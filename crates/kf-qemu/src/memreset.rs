// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The device reset's memory stage as a testable driver** (2026-10-18, review of branch
//! `claude/bootlayer-20261018`). The VA thread used to run `reset_memory_life` whole on every pass,
//! so a scrub the host refused re-ran the retire, the root zero and the boot-framebuffer zero every
//! 50 ms under a firmware that might already be drawing. The stage is now three steps with ONE
//! owner, and only the scrub is retried:
//!
//! 1. **prepare** (once per reset): retire the guest's mirrors and sink every BAR1/BAR2/PRAMIN CPU
//!    view to scratch — so no view of the old life's bytes survives into the scrub, even if the
//!    vCPUs resume at the main loop's 2 s bound with the gate still closed;
//! 2. **scrub** (only when owed; retried with a bounded back-off): the whole store, GPU-side, the
//!    completion awaited — a refusal is returned, never acknowledged;
//! 3. **finish** (once): our roots and the boot framebuffer zeroed and awaited, the windows back to
//!    the boot view, the line for the log.
//!
//! The reset is acknowledged only when [`Driver::step`] returns [`Step::Done`]. What happens at the
//! 2 s bound is the reset gate's existing contract (`V3_DEVICE_RESET.md` §3, §6 items 1 and 12): the
//! main loop stops waiting, the vCPUs resume, BAR0 stays gated (writes dropped, reads all ones) and
//! the stage keeps retrying in the background — the device never opens on an unscrubbed store, and
//! because step 1 ran first the guest's CPU views of the store are scratch meanwhile.

use std::time::{Duration, Instant};

/// The memory plane, as the driver needs it.
pub trait Ops {
    /// Retire the old life's mirrors and sink every CPU view to scratch. Idempotent. Returns a note
    /// for the log line.
    fn prepare(&mut self) -> String;
    /// Zero the whole store on the GPU and await the completion. Returns a note, or the refusal.
    ///
    /// # Errors
    /// The host's words; the store is NOT clean.
    fn scrub(&mut self) -> Result<String, String>;
    /// Our roots and the boot framebuffer zeroed (awaited), the windows back to the boot view.
    fn finish(&mut self, prepared: &str) -> Finished;
}

/// What [`Ops::finish`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    /// The boot log's line.
    pub line: String,
    /// The boot framebuffer's zero was accepted and awaited.
    pub boot_zeroed: bool,
}

/// One pass of the driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A retry is not due yet (back-off): nothing was done.
    Waiting,
    /// The scrub was refused: the reset must NOT be acknowledged. `attempt` counts this episode.
    Refused {
        /// The host's words.
        why: String,
        /// This episode's failure count (1-based).
        attempt: u64,
        /// When the next attempt is due.
        retry_in: Duration,
    },
    /// Every step completed: acknowledge the reset.
    Done {
        /// The boot log's line, with the scrub's note.
        line: String,
        /// See [`Finished::boot_zeroed`].
        boot_zeroed: bool,
    },
}

/// The back-off after the `n`th consecutive refusal: 50 ms, doubling, capped at 1 s.
#[must_use]
pub fn backoff(n: u64) -> Duration {
    // RED STEP (tests first)
    let _ = n;
    Duration::ZERO
}

/// The driver's state across the VA thread's passes.
#[derive(Debug, Default)]
pub struct Driver {
    prepared: Option<String>,
    scrub_owed: bool,
    retry_at: Option<Instant>,
    failures: u64,
    episode: u64,
}

impl Driver {
    /// One pass for reset `epoch`. `scrub`: the reset asked for the scrub (it stays owed across
    /// passes until it completed).
    pub fn step(&mut self, epoch: u64, scrub: bool, ops: &mut impl Ops, now: Instant) -> Step {
        // RED STEP (tests first): everything in one go, no retry logic
        let _ = (epoch, scrub, now);
        let p = ops.prepare();
        let note = ops.scrub().unwrap_or_default();
        let f = ops.finish(&p);
        Step::Done {
            line: format!("{}{note}", f.line),
            boot_zeroed: f.boot_zeroed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A memory plane that records every call; `scrub_fails` refusals are served before it works.
    #[derive(Default)]
    struct Fake {
        calls: Vec<&'static str>,
        scrub_fails: u32,
        boot_ok: bool,
    }

    impl Ops for Fake {
        fn prepare(&mut self) -> String {
            self.calls.push("prepare");
            "prep".to_string()
        }
        fn scrub(&mut self) -> Result<String, String> {
            self.calls.push("scrub");
            if self.scrub_fails > 0 {
                self.scrub_fails -= 1;
                return Err("cuMemsetD8: refused".to_string());
            }
            Ok("; scrubbed".to_string())
        }
        fn finish(&mut self, prepared: &str) -> Finished {
            assert_eq!(prepared, "prep", "finish gets prepare's note");
            self.calls.push("finish");
            Finished {
                line: "reset line".to_string(),
                boot_zeroed: self.boot_ok,
            }
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// ★ Review (M1b): an owed scrub RUNS, between the prepare and the finish — views sunk first, the
    /// roots and the boot zero after.
    #[test]
    fn an_owed_scrub_runs_between_prepare_and_finish() {
        let mut d = Driver::default();
        let mut f = Fake { boot_ok: true, ..Fake::default() };
        let s = d.step(1, true, &mut f, t0());
        assert_eq!(f.calls, ["prepare", "scrub", "finish"]);
        assert!(matches!(s, Step::Done { boot_zeroed: true, .. }), "{s:?}");
        if let Step::Done { line, .. } = s {
            assert!(line.contains("reset line") && line.contains("scrubbed"), "{line}");
        }
    }

    /// ★ Review (M1b): a reset that owes no scrub never scrubs (an in-guest reset, an S3 wake).
    #[test]
    fn a_reset_that_owes_no_scrub_does_not_scrub() {
        let mut d = Driver::default();
        let mut f = Fake::default();
        assert!(matches!(d.step(1, false, &mut f, t0()), Step::Done { .. }));
        assert_eq!(f.calls, ["prepare", "finish"]);
    }

    /// ★ Review (M4b): a refused scrub is NEVER acknowledged — no finish, no Done — and only the
    /// scrub is retried: the prepare ran once, the roots and the boot framebuffer were not touched.
    #[test]
    fn a_refused_scrub_is_not_acknowledged_and_only_the_scrub_is_retried() {
        let mut d = Driver::default();
        let mut f = Fake { scrub_fails: 2, boot_ok: true, ..Fake::default() };
        let mut now = t0();
        let s1 = d.step(1, true, &mut f, now);
        assert!(matches!(s1, Step::Refused { attempt: 1, .. }), "{s1:?}");
        assert_eq!(f.calls, ["prepare", "scrub"], "no finish after a refusal");
        // back-off: asked again at once, nothing is done
        assert_eq!(d.step(1, true, &mut f, now), Step::Waiting);
        assert_eq!(f.calls, ["prepare", "scrub"]);
        now += Duration::from_secs(2);
        let s2 = d.step(1, true, &mut f, now);
        assert!(matches!(s2, Step::Refused { attempt: 2, .. }), "{s2:?}");
        now += Duration::from_secs(2);
        let s3 = d.step(1, true, &mut f, now);
        assert!(matches!(s3, Step::Done { .. }), "{s3:?}");
        assert_eq!(
            f.calls,
            ["prepare", "scrub", "scrub", "scrub", "finish"],
            "prepare and finish ran once; only the scrub was retried"
        );
    }

    /// The owed scrub survives a pass that did not ask for it again (the reset asked once).
    #[test]
    fn the_owed_scrub_survives_the_retry() {
        let mut d = Driver::default();
        let mut f = Fake { scrub_fails: 1, boot_ok: true, ..Fake::default() };
        let now = t0();
        assert!(matches!(d.step(1, true, &mut f, now), Step::Refused { .. }));
        let later = now + Duration::from_secs(2);
        assert!(matches!(d.step(1, false, &mut f, later), Step::Done { .. }));
        assert_eq!(f.calls.iter().filter(|c| **c == "scrub").count(), 2);
    }

    /// Review (1): the failure count is per episode — a new reset starts at attempt 1, so its early
    /// lines are not silent — and a new reset re-prepares.
    #[test]
    fn the_failure_count_is_per_episode() {
        let mut d = Driver::default();
        let mut f = Fake { scrub_fails: 3, boot_ok: true, ..Fake::default() };
        let mut now = t0();
        for want in 1..=2u64 {
            let s = d.step(1, true, &mut f, now);
            assert!(matches!(s, Step::Refused { attempt, .. } if attempt == want), "{s:?}");
            now += Duration::from_secs(2);
        }
        let s = d.step(2, true, &mut f, now);
        assert!(matches!(s, Step::Refused { attempt: 1, .. }), "a new episode starts at 1: {s:?}");
        assert_eq!(f.calls.iter().filter(|c| **c == "prepare").count(), 2, "{:?}", f.calls);
    }

    /// The back-off: 50 ms, doubling, capped at one second.
    #[test]
    fn the_backoff_doubles_to_a_cap() {
        let ms: Vec<u128> = (1..=7).map(|n| backoff(n).as_millis()).collect();
        assert_eq!(ms, [50, 100, 200, 400, 800, 1000, 1000]);
    }
}
