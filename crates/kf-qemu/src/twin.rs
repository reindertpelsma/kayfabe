// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ P1+P2 inc D (`docs/design/V3_P1P2_TSPACE.md` §4.2) — **the per-twin state: ONE atomic word.**
//!
//! A mirrored space is `Unclassified`, `User(n)` (n live passthrough channels the guest created
//! non-kernel) or `Kernel` (a Translated channel — the guest kernel's — runs in it, or its client
//! is one of the guest RM's internal clients). Before this, `mirror.live` and `kernel_vas` were
//! separate atomics flipped on different threads, so a passthrough statement, a Translated
//! statement, the deferred passthrough birth and a walk could interleave into a live USER channel
//! in a space holding privileged guest leaves.
//!
//! Transitions are compare-and-swaps made on the STATEMENT path, in statement order:
//! - a passthrough birth: `Unclassified | User(n)` → `User(n+1)`; REFUSED by name in `Kernel`;
//! - a Translated birth: `Unclassified` → `Kernel` (and `Kernel` stays); REFUSED by name in
//!   `User(n > 0)`;
//! - a passthrough free: `User(n)` → `User(n-1)` (`User(0)` is `Unclassified`);
//! - `Kernel` is sticky until the space retires; a recycled space is classified afresh.
//!
//! The walker reads the word with ONE atomic load when it commits a privileged leaf and places it
//! only in `Kernel` — so a privileged leaf is never placed in a space a user channel can run in,
//! and no lock is added to the VA thread's path.
//!
//! ⚠ T-mode only (`KF3_TSPACE=1`). On the default path a Translated birth still marks the space
//! kernel unconditionally ([`TwinState::force_kernel`]) and passthrough births do not count —
//! today's behaviour, unchanged.

use std::sync::atomic::{AtomicU64, Ordering};

/// The `Kernel` bit; the low bits count live user channels.
const KERNEL: u64 = 1 << 63;

/// ★ The per-twin state word. See the module docs.
#[derive(Debug, Default)]
pub struct TwinState(AtomicU64);

/// Why a birth was refused by the twin state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwinRefusal {
    /// A passthrough birth in a guest-KERNEL space.
    UserInKernelSpace,
    /// A Translated birth in a space with this many live user channels.
    KernelInUserSpace(u64),
}

impl TwinState {
    /// The state a space starts in: `Kernel` for one of the guest RM's own internal clients (a
    /// handle range no guest process can hold, `kf_rm::chanlink::is_rm_internal_client`),
    /// `Unclassified` otherwise.
    #[must_use]
    pub fn for_kernel(kernel: bool) -> TwinState {
        TwinState(AtomicU64::new(if kernel { KERNEL } else { 0 }))
    }

    /// The space is the guest kernel's: privileged leaves may be mirrored. ONE atomic load.
    #[must_use]
    pub fn is_kernel(&self) -> bool {
        self.0.load(Ordering::Acquire) & KERNEL != 0
    }

    /// Live user channels counted.
    #[must_use]
    pub fn users(&self) -> u64 {
        self.0.load(Ordering::Acquire) & !KERNEL
    }

    /// ★ A passthrough birth (T-mode): `User(n)` → `User(n+1)`.
    ///
    /// # Errors
    /// [`TwinRefusal::UserInKernelSpace`] — refused, never waited on.
    pub fn try_user(&self) -> Result<(), TwinRefusal> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
                (s & KERNEL == 0).then_some(s + 1)
            })
            .map(|_| ())
            .map_err(|_| TwinRefusal::UserInKernelSpace)
    }

    /// A passthrough channel counted by [`TwinState::try_user`] is gone.
    pub fn user_done(&self) {
        let _ = self.0.try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
            (s & !KERNEL > 0).then(|| s - 1)
        });
    }

    /// ★ A Translated birth (T-mode): `Unclassified` → `Kernel`; `Kernel` stays. Returns whether
    /// this birth made the space kernel.
    ///
    /// # Errors
    /// [`TwinRefusal::KernelInUserSpace`] while any user channel is live.
    pub fn try_kernel(&self) -> Result<bool, TwinRefusal> {
        let prev = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
                (s & !KERNEL == 0).then_some(KERNEL)
            })
            .map_err(|s| TwinRefusal::KernelInUserSpace(s & !KERNEL))?;
        Ok(prev & KERNEL == 0)
    }

    /// The default path's unconditional flip (today's `kernel_vas.swap(true)`): returns whether the
    /// space was already kernel.
    pub fn force_kernel(&self) -> bool {
        self.0.fetch_or(KERNEL, Ordering::AcqRel) & KERNEL != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One operation of the interleaving model.
    #[derive(Debug, Clone, Copy)]
    enum Op {
        /// A passthrough statement (counts at the statement).
        UserStatement,
        /// Its deferred birth fails: the count is given back.
        UserBirthFails,
        /// A passthrough channel is freed.
        UserFree,
        /// A Translated statement.
        KernelStatement,
        /// A walk that commits a privileged leaf (placed only if the state says `Kernel`).
        Walk,
    }

    /// ★★ §7 test 16 — **`twin_state_cas`**: EVERY interleaving of up to six operations — passthrough
    /// statements, deferred births that fail, frees, Translated statements and walks — never leaves
    /// a live `User(n > 0)` space holding a privileged leaf, and never lets a user channel be born
    /// into a space that holds one.
    #[test]
    fn twin_state_cas() {
        let ops = [
            Op::UserStatement,
            Op::UserBirthFails,
            Op::UserFree,
            Op::KernelStatement,
            Op::Walk,
        ];
        let mut seqs = vec![Vec::new()];
        let mut checked = 0u64;
        for _ in 0..6 {
            let mut next = Vec::new();
            for s in &seqs {
                for &o in &ops {
                    let mut t: Vec<Op> = s.clone();
                    t.push(o);
                    next.push(t);
                }
            }
            seqs = next;
            for s in &seqs {
                let st = TwinState::default();
                let mut users = 0u64; // live user channels, per the model
                let mut privileged = false; // a privileged leaf is placed
                for &o in s {
                    match o {
                        Op::UserStatement => {
                            if st.try_user().is_ok() {
                                assert!(!privileged, "{s:?}: a user birth into a privileged space");
                                users += 1;
                            }
                        }
                        Op::UserBirthFails | Op::UserFree => {
                            if users > 0 {
                                users -= 1;
                                st.user_done();
                            }
                        }
                        Op::KernelStatement => {
                            let _ = st.try_kernel();
                        }
                        Op::Walk => {
                            if st.is_kernel() {
                                privileged = true;
                            }
                        }
                    }
                    assert_eq!(st.users(), users, "{s:?}");
                    assert!(
                        !(privileged && users > 0),
                        "{s:?}: a live user channel in a space holding a privileged leaf"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 10_000);
    }

    /// ★ Reset on recycle: a recycled space is classified afresh — `Kernel` only for an RM-internal
    /// client — and `Kernel` is sticky until then.
    #[test]
    fn kernel_is_sticky_and_a_recycled_space_is_reclassified() {
        let st = TwinState::default();
        assert_eq!(st.try_kernel(), Ok(true));
        assert_eq!(st.try_kernel(), Ok(false));
        assert_eq!(st.try_user(), Err(TwinRefusal::UserInKernelSpace));
        st.user_done();
        assert!(st.is_kernel(), "sticky");
        let recycled = TwinState::for_kernel(false);
        assert!(!recycled.is_kernel());
        assert_eq!(recycled.try_user(), Ok(()));
        assert_eq!(
            recycled.try_kernel(),
            Err(TwinRefusal::KernelInUserSpace(1))
        );
        recycled.user_done();
        assert_eq!(recycled.try_kernel(), Ok(true));
        assert!(TwinState::for_kernel(true).is_kernel());
        // The default path's flip.
        let legacy = TwinState::default();
        assert_eq!(legacy.try_user(), Ok(()));
        assert!(!legacy.force_kernel(), "today's flip ignores live users");
        assert!(legacy.is_kernel());
    }
}
