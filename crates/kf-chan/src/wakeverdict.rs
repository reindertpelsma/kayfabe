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

#[cfg(test)]
mod tests {
    use super::wake_verdict;

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
        assert_eq!(wake_verdict(u32::MAX - 3, Some(2), true), "host fence not reached");
    }
}
