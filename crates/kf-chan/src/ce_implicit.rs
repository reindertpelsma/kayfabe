// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-10-19 — WHICH rings get the CE-implicit binding ([`crate::tmode::TState::ce_object`]).
//!
//! The decision is a pure function of facts the HOST holds, so the birth route cannot be talked
//! into it by a guest: the channel is guest-KERNEL work (facts guest userspace cannot produce:
//! `kernel_client && !user_work`), its engine is a copy engine, and the ring's own host channel
//! holds a copy-engine object of a class the T-mode CE table knows. Anything else gets `0` (the
//! strict `NoCeObject` behaviour).

/// The CE class a Translated ring decodes copy-engine methods against when the guest sends no
/// `SET_OBJECT`, or `0` for none.
///
/// - `kernel_work` — the channel is guest-KERNEL work, not a user-tier (Passthrough) channel;
/// - `copy_engine` — the guest channel's engine is a copy engine;
/// - `host_ce` — the class of the CE object the ring's host channel holds
///   ([`crate::host::HostRing::ce_object_class`]), `None` on a graphics or video ring.
#[must_use]
pub fn implicit_ce_class(kernel_work: bool, copy_engine: bool, host_ce: Option<u32>) -> u32 {
    let _ = (kernel_work, copy_engine, host_ce);
    0 // STUB (tests first)
}

#[cfg(test)]
mod tests {
    use super::implicit_ce_class;

    #[test]
    fn a_kernel_copy_engine_ring_gets_its_host_ce_class() {
        for class in [0xC5B5, 0xC6B5, 0xC7B5, 0xC8B5, 0xC9B5, 0xCAB5] {
            assert_eq!(implicit_ce_class(true, true, Some(class)), class);
        }
    }

    #[test]
    fn a_user_tier_channel_never_gains_it() {
        assert_eq!(implicit_ce_class(false, true, Some(0xC7B5)), 0);
    }

    #[test]
    fn a_non_copy_engine_channel_never_gains_it() {
        assert_eq!(implicit_ce_class(true, false, Some(0xC7B5)), 0);
        assert_eq!(implicit_ce_class(false, false, Some(0xC7B5)), 0);
    }

    #[test]
    fn a_ring_with_no_host_ce_object_or_a_foreign_class_gets_nothing() {
        assert_eq!(implicit_ce_class(true, true, None), 0);
        for class in [0, 0xC9B0, 0xC997, 0xC7B4, 0xC7B6, 0xDEAD, u32::MAX] {
            assert_eq!(implicit_ce_class(true, true, Some(class)), 0, "{class:#x}");
        }
    }
}
