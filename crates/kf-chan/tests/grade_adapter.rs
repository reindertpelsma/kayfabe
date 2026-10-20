//! 2026-10-20 -- **The grader's lineage adapter** (`docs/design/V3_RELAY_GRADER_20261020.md`). Included with `#[path]` by the
//! `grade_*` test targets; not a test target of its own (no `#[test]` here).
//!
//! The grader's tests (`grade_model.rs`, `grade_locks.rs`, `grade_static.rs`, `grade_stress.rs`) are lineage-NEUTRAL: every
//! decision a relay implementation makes is asked of this file, and this file answers with the implementation's OWN pure
//! functions (production code: `kf_chan::user_loop::*`, `kf_chan::worker::LoopPolicy`, `kf_chan::handoff::*`) composed the way
//! the implementation's glue (`crates/kf-qemu/src/userpump.rs`, which cannot be built without a GPU) composes them (CODE READ,
//! pinned by `PINS` below: `grade_static::g9_adapter_pins_match_the_glue` fails when the glue drifts).
//!
//! Lineages (selected by `--cfg kf_grade_lineage="..."`, which `scripts/relay_validity.sh` sets; the default is `simple`, so a
//! plain `cargo test -p kf-chan` grades the branch it is on):
//! * `simple` -- the one-loop relay (`claude/relay-simple-20261020`): `user_loop::look`, `RelayLoop::unsourced`, no budget, a
//!   busy pass always evaluates, infinite park knob `worker::set_park_net`.
//! * `look_every` -- the bounded-fairness fix in the old structure (`cfdcaf73b`, `82830cd5e`, `claude/relay-fairness-2-20261020`):
//!   `may_retry` / `CONTENTION_RETRIES`, `LoopPolicy` with `LOOK_EVERY`, the bounded pre-park `SPIN`, the 50 ms backstop.
//! * `budget` -- before the fairness fix (`0d325edab`): as `look_every` but a busy worker never looks.
//!
//! What a NEW implementation must provide: a `cfg` arm here answering the functions below (`look`, `after_pass`, `loop_shape`,
//! `contended_leaves_demand`, `contended_wait`, `unsourced`, `PARK_KNOB`, `BUDGET`, `PINS`, `GLUE_FILES`, `LOOP_FILES`).
#![allow(dead_code, unexpected_cfgs, missing_docs)]

/// Why a look happens (the abstraction of `user_loop::Peek` across lineages).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Why {
    /// The level evaluation of the loop / the idle spin.
    Spin,
    /// After an engine / completion event was dispatched.
    Event,
}

/// The abstract kinds of a recorded wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AWait {
    /// A marker wait: announced by the marker's engine interrupt.
    Marker,
    /// A pop wait: no interrupt announces it (carried).
    Popped,
    /// The old lineages' `Wait::Contended` (a lost lock recorded as a wait).
    Contended,
}

/// What one look at one relay sees (level state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub why: Why,
    /// `Cell::pending()` (durable demand).
    pub demand: bool,
    /// The record `try_lock` of the look failed.
    pub record_busy: bool,
    /// `Cell::is_waiting()` -- readable without the record lock.
    pub waiting_flag: bool,
    /// The recorded wait (meaningful only when `!record_busy`).
    pub wait: Option<AWait>,
    /// The hardware words satisfy the wait (meaningful only with a wait and `!storage_busy`).
    pub satisfied: bool,
    /// The storage `try_lock` of the look failed (words unreadable).
    pub storage_busy: bool,
    /// `Cell::contended_run()`.
    pub crun: u32,
}

/// What the look does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    Leave,
    Ring { clear_wait: bool },
}

/// After a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum After {
    Continue,
    Look,
    Idle,
}

/// How the loop reaches its blocking point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    /// ONE loop: evaluate every iteration, `epoll_wait(0)` while anything is outstanding without a wake source, else block.
    OneLoop,
    /// The old loop: a bounded pre-park spin of `spin` looks while some relay waits, then a park.
    SpinThenPark { spin: u32 },
}

// ------------------------------------------------------------------------------------------------------------------------------
// simple (the default)

#[cfg(not(any(kf_grade_lineage = "look_every", kf_grade_lineage = "budget")))]
mod imp {
    use super::*;
    use kf_chan::user_loop::{self as ul, Look, LookIn, Wait};
    use kf_chan::worker::{AfterPass, LoopPolicy};

    pub const LINEAGE: &str = "simple";
    /// The park can be made INFINITE (`worker::set_park_net(false)`).
    pub const PARK_KNOB: bool = true;
    /// No retry budget.
    pub const BUDGET: Option<u32> = None;

    pub fn set_infinite_park() {
        kf_chan::worker::set_park_net(false);
    }

    /// MIRROR of `ChanPlane::user_ring_peek_one` (userpump.rs): `look_needed`, then `user_loop::look` on what the look read.
    pub fn look(v: View) -> Verdict {
        let why = match v.why {
            Why::Spin => ul::Peek::Spin,
            Why::Event => ul::Peek::Event,
        };
        if !ul::look_needed(why, v.demand, v.waiting_flag) {
            return Verdict::Leave;
        }
        let to_wait = |w: AWait| match w {
            AWait::Marker | AWait::Contended => Wait::Marker { target: 1 },
            AWait::Popped => Wait::Popped { target: 1 },
        };
        let i = if v.record_busy {
            LookIn {
                demand: v.demand,
                record_busy: true,
                wait: v.waiting_flag.then_some(Wait::Marker { target: u64::MAX }),
                words: None,
                fetched: 0,
                get_follows_pops: false,
            }
        } else {
            LookIn {
                demand: v.demand,
                record_busy: false,
                wait: v.wait.map(to_wait),
                words: if v.wait.is_none() || v.storage_busy {
                    None
                } else if v.satisfied {
                    Some((1, 1))
                } else {
                    Some((0, 0))
                },
                fetched: 0,
                get_follows_pops: kf_chan::user_turn::guest_get_follows_pops(),
            }
        };
        match ul::look(i) {
            Look::Leave => Verdict::Leave,
            Look::Ring { clear_wait, .. } => Verdict::Ring { clear_wait },
        }
    }

    pub type Policy = LoopPolicy;
    pub fn policy() -> Policy {
        LoopPolicy::default()
    }
    pub fn after_pass(p: &mut Policy, served: usize) -> After {
        match p.after_pass(served) {
            AfterPass::Continue => After::Continue,
            AfterPass::Look => After::Look,
            AfterPass::Idle => After::Idle,
        }
    }
    pub fn loop_shape() -> Shape {
        Shape::OneLoop
    }
    /// `user_ring_arm`'s `Need::Contended` arm: `record.request()` (the demand flag stays set: the retry set).
    pub fn contended_leaves_demand() -> bool {
        true
    }
    /// The wait a contended turn records (none: a lost lock is demand, not a wait).
    pub fn contended_wait() -> Option<AWait> {
        None
    }
    /// MIRROR of `user_ring_unsourced`: demand, or a carried (pop) wait.
    pub fn unsourced(demand: bool, waiting: bool, carried: bool, act_hold: bool) -> bool {
        // 2026-10-20 G5: demand while an ACT holds the record is covered (the act's release rings)
        (demand && !act_hold) || (waiting && carried)
    }
    /// The lines of the glue the mirrors above transliterate (file relative to the workspace root, text).
    pub const PINS: &[(&str, &str)] = &[
        ("crates/kf-qemu/src/userpump.rs", "return matches!(look(i), Look::Ring { .. }) && ring(self);"),
        ("crates/kf-qemu/src/userpump.rs", "record.contended_note();\n                let _ = record.request();"),
        ("crates/kf-qemu/src/userpump.rs", "(record.pending() && !record.act_held())"),
        ("crates/kf-chan/src/worker.rs", "let spin = must_spin(n, rang, relay);"),
    ];
}

// ------------------------------------------------------------------------------------------------------------------------------
// look_every and budget (the old structure)

#[cfg(any(kf_grade_lineage = "look_every", kf_grade_lineage = "budget"))]
mod imp {
    use super::*;
    use kf_chan::user_loop::{self as ul};
    use kf_chan::worker::{AfterPass, LoopPolicy};

    #[cfg(kf_grade_lineage = "look_every")]
    pub const LINEAGE: &str = "look_every";
    #[cfg(kf_grade_lineage = "budget")]
    pub const LINEAGE: &str = "budget";
    /// The 50 ms net is not switchable: it is a logic path here (`Peek::Backstop`). The grader runs as if it were infinite and
    /// counts every timeout that finds work as a failure.
    pub const PARK_KNOB: bool = false;
    pub const BUDGET: Option<u32> = Some(ul::CONTENTION_RETRIES);

    pub fn set_infinite_park() {}

    /// MIRROR of the old `ChanPlane::user_ring_peek_one` (userpump.rs at 82830cd5e, 2865-2980), minus the backstop branch (the
    /// grader never fires the timer).
    pub fn look(v: View) -> Verdict {
        let why = match v.why {
            Why::Spin => ul::Peek::Spin,
            Why::Event => ul::Peek::Event,
        };
        if !ul::look_needed(why, v.demand, v.waiting_flag) {
            return Verdict::Leave;
        }
        let retry_ok = ul::may_retry(v.crun, why);
        if v.record_busy {
            return if retry_ok {
                Verdict::Ring { clear_wait: false }
            } else {
                Verdict::Leave
            };
        }
        if let Some(w) = v.wait {
            let satisfied = v.storage_busy || v.satisfied || w == AWait::Contended;
            if satisfied && (retry_ok || w != AWait::Contended) {
                return Verdict::Ring { clear_wait: true };
            }
        }
        if v.demand && retry_ok {
            return Verdict::Ring { clear_wait: false };
        }
        Verdict::Leave
    }

    pub type Policy = LoopPolicy;
    pub fn policy() -> Policy {
        LoopPolicy::default()
    }
    pub fn after_pass(p: &mut Policy, served: usize) -> After {
        match p.after_pass(served) {
            AfterPass::Continue => After::Continue,
            AfterPass::Look => After::Look,
            AfterPass::Idle => After::Idle,
        }
    }
    pub fn loop_shape() -> Shape {
        Shape::SpinThenPark { spin: ul::SPIN }
    }
    /// The old `user_ring_arm`'s `Need::Contended` arm: `contended_note` only (the demand was set by `enter()` and the turn
    /// consumed it with `retry.swap(false)`).
    pub fn contended_leaves_demand() -> bool {
        false
    }
    pub fn contended_wait() -> Option<AWait> {
        Some(AWait::Contended)
    }
    /// The old loop has no unsourced notion: it parks after the spin whatever is outstanding.
    pub fn unsourced(_demand: bool, _waiting: bool, _carried: bool, _act_hold: bool) -> bool {
        false
    }
    pub const PINS: &[(&str, &str)] = &[
        ("crates/kf-qemu/src/userpump.rs", "let retry_ok = kf_chan::user_loop::may_retry(record.contended_run(), why);"),
        ("crates/kf-qemu/src/userpump.rs", "return retry_ok && ring(self);"),
        ("crates/kf-qemu/src/userpump.rs", "if demand && retry_ok {"),
        ("crates/kf-chan/src/user_loop.rs", "matches!(why, Peek::Backstop) || run < CONTENTION_RETRIES"),
    ];
}

pub use imp::*;

/// The modules the static audits (G9) read: the worker loop and the relay's decision vocabulary, and the glue.
pub const LOOP_FILES: &[&str] = &[
    "crates/kf-chan/src/worker.rs",
    "crates/kf-chan/src/user_loop.rs",
];
pub const GLUE_FILES: &[&str] = &[
    "crates/kf-qemu/src/userpump.rs",
    "crates/kf-qemu/src/userrelay.rs",
];

/// The workspace root (the test runs with `CARGO_MANIFEST_DIR` = `crates/kf-chan`).
pub fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// A workspace file as text (empty when absent).
pub fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_default()
}
