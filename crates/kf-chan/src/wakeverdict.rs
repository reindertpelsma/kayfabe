// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The completion probe's verdict on a fence that is "in flight" for too long
//! (`KF3_COMPLETION_PROBE`, diagnostic only, never a decision input).
//!
//! A Translated channel's fence is seen complete only when its pump runs and reads the ring's fence
//! word. The probe's dump already prints that word (`host_cursors().2`) next to the in-flight
//! sequence, but a reader had to compare them by eye. TUR1 (lease 57882297, TU106, Windows 580.88)
//! produced a dump with the host ring drained (`GP_GET == GP_PUT`) and the fence word equal to the
//! in-flight sequence, so the host had finished and nothing had pumped the channel since: a lost
//! wake, not a stuck engine. This names which case a dump is.

/// What a probe dump of a Translated channel says about its oldest in-flight fence.
///
/// `fence_word` is the ring's fence word (what the host engine last released), `head` the oldest
/// in-flight sequence, `marked` whether the channel's token is in the session's completion set
/// (the set a wake rings).
#[must_use]
pub fn wake_verdict(fence_word: u32, head: Option<u32>, marked: bool) -> &'static str {
    match head {
        None => "no fence in flight",
        Some(seq) if !crate::host::reached(fence_word, seq) => "host fence not reached",
        Some(_) if marked => {
            "HOST-DONE-NOT-SEEN marked: the host released the fence and no pump has run since \
             (a lost wake)"
        }
        Some(_) => {
            "HOST-DONE-NOT-SEEN UNMARKED: the host released the fence and the token is not in the \
             completion set, so no wake would ring it"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{lost_wake_cause, wake_verdict};

    #[test]
    fn nothing_in_flight_is_nothing_to_say() {
        assert_eq!(wake_verdict(5, None, false), "no fence in flight");
    }

    #[test]
    fn the_fence_word_short_of_the_head_means_the_host_is_not_done() {
        assert_eq!(wake_verdict(764, Some(765), true), "host fence not reached");
    }

    #[test]
    fn a_reached_fence_that_is_marked_is_a_lost_wake() {
        let v = wake_verdict(765, Some(765), true);
        assert!(v.starts_with("HOST-DONE-NOT-SEEN"), "{v}");
        assert!(v.contains("marked"), "{v}");
        assert!(!v.contains("UNMARKED"), "{v}");
    }

    #[test]
    fn a_reached_fence_that_is_not_marked_is_a_lost_mark() {
        let v = wake_verdict(766, Some(765), false);
        assert!(v.starts_with("HOST-DONE-NOT-SEEN"), "{v}");
        assert!(v.contains("UNMARKED"), "{v}");
    }

    #[test]
    fn the_comparison_wraps_like_the_ring_sequence() {
        let v = wake_verdict(2, Some(u32::MAX - 1), true);
        assert!(v.starts_with("HOST-DONE-NOT-SEEN"), "{v}");
        assert_eq!(
            wake_verdict(u32::MAX - 3, Some(2), true),
            "host fence not reached"
        );
    }
    #[test]
    fn no_wake_since_the_fence_was_queued_means_the_interrupt_never_came() {
        let c = lost_wake_cause(None, Some(300), Some(10), 5);
        assert!(c.starts_with("NO-WAKE"), "{c}");
        // a wake that is older than the last pump of this channel changes nothing
        let c = lost_wake_cause(Some(900), Some(300), Some(10), 5);
        assert!(c.starts_with("NO-WAKE"), "{c}");
    }

    #[test]
    fn a_wake_after_the_last_pump_that_did_not_pump_is_a_lost_ring() {
        let c = lost_wake_cause(Some(100), Some(300), Some(10), 5);
        assert!(c.starts_with("WAKE-NOT-PUMPED"), "{c}");
        let c = lost_wake_cause(Some(100), None, None, 5);
        assert!(c.starts_with("WAKE-NOT-PUMPED"), "{c}");
    }

    #[test]
    fn a_pump_after_the_wake_that_read_a_short_fence_is_a_stale_read() {
        // the last wake 300 ms ago, the last pump 200 ms ago, and it read word 4 for head 5
        let c = lost_wake_cause(Some(300), Some(200), Some(4), 5);
        assert!(c.starts_with("STALE-READ-AT-WAKE"), "{c}");
    }

    #[test]
    fn a_pump_that_saw_the_fence_is_not_a_lost_wake() {
        let c = lost_wake_cause(Some(300), Some(200), Some(5), 5);
        assert!(c.starts_with("PUMP-SAW-IT"), "{c}");
    }
}
