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
//! Transitions are compare-and-swaps made on the STATEMENT path, in statement order — and a free
//! is FINISHED only when host RM has really freed the channel (review fix 2026-10-04):
//! - a passthrough birth: `Unclassified | User(n)` → `User(n+1)`; REFUSED by name in `Kernel`;
//! - a Translated birth: `Unclassified` → `Kernel` (and `Kernel` stays); REFUSED by name while any
//!   user channel is live OR still being freed — the two are told apart ([`TwinRefusal`]);
//! - a passthrough free STATEMENT moves one user from live to FREEING ([`TwinState::user_freeing`]);
//!   the deferred host free then ends it ([`TwinState::user_released`]) — but only when host RM
//!   freed the channel: a refused free leaves it counted for the life of the space, so a space
//!   whose user channel may still run can never become `Kernel`;
//! - a birth whose deferred host birth failed gives its count back ([`TwinState::user_done`]);
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

/// The `Kernel` bit.
const KERNEL: u64 = 1 << 63;
/// Live user channels: bits 31:0.
const LIVE: u64 = 0xFFFF_FFFF;
/// One user channel being freed (its free statement seen, host RM not done): bits 62:32.
const FREEING_ONE: u64 = 1 << 32;
/// The freeing count's field.
const FREEING: u64 = !KERNEL & !LIVE;

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
    /// ★ A Translated birth in a space with no live user channel but this many whose host free
    /// has not finished (or was refused) — still channels host RM may run.
    KernelWhileUsersFree(u64),
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
        self.0.load(Ordering::Acquire) & LIVE
    }

    /// User channels whose free statement was seen and whose host free has not finished.
    #[must_use]
    pub fn freeing(&self) -> u64 {
        (self.0.load(Ordering::Acquire) & FREEING) >> 32
    }

    /// ★ A passthrough birth (T-mode): `User(n)` → `User(n+1)`.
    ///
    /// # Errors
    /// [`TwinRefusal::UserInKernelSpace`] — refused, never waited on.
    pub fn try_user(&self) -> Result<(), TwinRefusal> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
                (s & KERNEL == 0 && s & LIVE < LIVE).then_some(s + 1)
            })
            .map(|_| ())
            .map_err(|_| TwinRefusal::UserInKernelSpace)
    }

    /// A passthrough channel counted by [`TwinState::try_user`] was never born (its deferred host
    /// birth failed): the count is given back.
    pub fn user_done(&self) {
        let _ = self.0.try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
            (s & LIVE > 0).then(|| s - 1)
        });
    }

    /// ★ The FREE STATEMENT of a passthrough channel (statement order): one user moves from live
    /// to freeing.
    pub fn user_freeing(&self) {
        let _ = self.0.try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
            (s & LIVE > 0 && s & FREEING < FREEING).then(|| s - 1 + FREEING_ONE)
        });
    }

    /// ★ Host RM FREED a channel [`TwinState::user_freeing`] counted: it is gone. ⊘ Never called
    /// for a refused free — that channel stays counted, and the space never becomes `Kernel`.
    pub fn user_released(&self) {
        let _ = self.0.try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
            (s & FREEING != 0).then(|| s - FREEING_ONE)
        });
    }

    /// ★ A Translated birth (T-mode): `Unclassified` → `Kernel`; `Kernel` stays. Returns whether
    /// this birth made the space kernel.
    ///
    /// # Errors
    /// [`TwinRefusal::KernelInUserSpace`] while any user channel is live,
    /// [`TwinRefusal::KernelWhileUsersFree`] while one is still being freed.
    pub fn try_kernel(&self) -> Result<bool, TwinRefusal> {
        let prev = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
                (s & !KERNEL == 0).then_some(KERNEL)
            })
            .map_err(|s| {
                if s & LIVE > 0 {
                    TwinRefusal::KernelInUserSpace(s & LIVE)
                } else {
                    TwinRefusal::KernelWhileUsersFree((s & FREEING) >> 32)
                }
            })?;
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
        /// A passthrough free statement (live -> freeing).
        UserFree,
        /// Its deferred host free succeeds (freeing -> gone).
        HostFreed,
        /// Its deferred host free is refused (stays freeing).
        HostFreeRefused,
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
            Op::HostFreed,
            Op::HostFreeRefused,
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
                let mut freeing = 0u64; // freed at the statement, host free not finished
                let mut refused = 0u64; // host free refused: the channel may still run
                let mut privileged = false; // a privileged leaf is placed
                for &o in s {
                    match o {
                        Op::UserStatement => {
                            if st.try_user().is_ok() {
                                assert!(!privileged, "{s:?}: a user birth into a privileged space");
                                users += 1;
                            }
                        }
                        Op::UserBirthFails => {
                            if users > 0 {
                                users -= 1;
                                st.user_done();
                            }
                        }
                        Op::UserFree => {
                            if users > 0 {
                                users -= 1;
                                freeing += 1;
                                st.user_freeing();
                            }
                        }
                        Op::HostFreed => {
                            if freeing > 0 {
                                freeing -= 1;
                                st.user_released();
                            }
                        }
                        Op::HostFreeRefused => {
                            if freeing > 0 {
                                freeing -= 1;
                                refused += 1; // never released
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
                    assert_eq!(st.freeing(), freeing + refused, "{s:?}");
                    assert!(
                        !(privileged && users + freeing + refused > 0),
                        "{s:?}: a user channel host RM may still run, in a space holding a privileged leaf"
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
        // ★ A free is FINISHED only when host RM freed the channel (review fix 2026-10-04).
        recycled.user_freeing();
        assert_eq!(
            recycled.try_kernel(),
            Err(TwinRefusal::KernelWhileUsersFree(1)),
            "the statement alone does not end the channel"
        );
        recycled.user_released();
        assert_eq!(recycled.try_kernel(), Ok(true));
        let refused = TwinState::default();
        assert_eq!(refused.try_user(), Ok(()));
        refused.user_freeing(); // the host free is then refused: never released
        assert_eq!(
            refused.try_kernel(),
            Err(TwinRefusal::KernelWhileUsersFree(1))
        );
        assert!(TwinState::for_kernel(true).is_kernel());
        // The default path's flip.
        let legacy = TwinState::default();
        assert_eq!(legacy.try_user(), Ok(()));
        assert!(!legacy.force_kernel(), "today's flip ignores live users");
        assert!(legacy.is_kernel());
    }
}
