// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **The one path a channel-class object reaches host RM by, and the bound that keeps it the
//! only one** (`docs/design/THE_CONSTRAINTS.md` §30; OWNER_RULINGS §P).
//!
//! Host RM stamps a channel's privilege once, at the channel-alloc ioctl, from the calling
//! thread's `capable(CAP_SYS_ADMIN)` (`ogkm-580: kernel_channel.c:277-291`, `escape.c:304`).
//! Every channel kf3 creates runs guest-authored work, so each one must be born `USER`. Two
//! things make that so, and both live in [`born_user`]:
//!
//! - **the mechanism**: the alloc is issued inside
//!   [`kf_linux_raw::capability::with_effective_cap_cleared_on`], with `CAP_SYS_ADMIN` cleared
//!   from the calling thread's effective set for that one call;
//! - **the reply check**: [`crate::channel::birth_privilege`] reads RM's verdict
//!   (`NVOS04_FLAGS_PRIVILEGED_CHANNEL`, 5:5) out of the reply and refuses the birth by name,
//!   freeing the channel.
//!
//! # Where the bound lives
//!
//! Every function in this crate that builds an `NV_ESC_RM_ALLOC` request calls
//! [`admit_alloc_class`] first. It refuses every class host RM constructs as a `KernelChannel`
//! ([`is_channel_class`]) unless the caller holds an [`InsideBirthPath`]. That token has a
//! private field, so it can only be made in this module, and only [`born_user`] makes one: it
//! hands it to the issuing closure, inside the bracket, and checks the reply after the closure
//! returns. So a new channel allocation anywhere in kf-host, including through the public
//! `HostRm::raw_alloc`, `raw_alloc_via` and `raw_alloc_nested`, is refused before any host call
//! (`CHANNEL_CLASS_OUTSIDE_BIRTH`, `0x4B75`) unless it goes through this function.
//!
//! ⊘ **Why refuse, rather than bracket and check every channel-class alloc wherever it is
//! issued.** The reply check is exact only for a request kf3 built itself: it reads `flags` at a
//! known offset of `NV_CHANNEL_ALLOC_PARAMS`, and only after making sure the request did not ask
//! for bit 5. A generic `raw_alloc` caller passes an arbitrary parameter block, so a check
//! applied there would have to re-derive the struct or accept a weaker reading. Refusing keeps
//! one audited birth path, makes a second one a named error at its first call, and is testable
//! without a GPU: the refusal happens before any host call.
//!
//! `crates/kf-host/tests/channel_birth_bound.rs` checks the shape in the source, and the unit
//! tests below check the behaviour against a simulated root thread on any runner.

use crate::channel::{BirthPrivilege, birth_privilege};
use crate::{
    CAP_BRACKET_REFUSED, CHANNEL_CLASS_OUTSIDE_BIRTH, PRIVILEGED_CHANNEL_REFUSED, RmError,
};
use kf_linux_raw::capability::{
    CAP_SYS_ADMIN, EffectiveBracket, ThreadCapOps, with_effective_cap_cleared_on,
};

/// ★ Proof that a channel-class allocation is being issued by [`born_user`]'s issuing closure.
///
/// The field is private, so no code outside this module can make one, and nothing here derives
/// `Clone`, `Copy` or `Default`: the closure only ever borrows the one [`born_user`] made, for
/// the length of the call.
#[derive(Debug)]
pub(crate) struct InsideBirthPath {
    _private: (),
}

/// What the channel-alloc call did about `CAP_SYS_ADMIN`, for the per-birth log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapNote {
    /// The bracket ran (`kf_linux_raw::capability::with_effective_cap_cleared_on`).
    Bracket(EffectiveBracket),
    /// `KF3_NEGCTL_SKIP_CAP_BRACKET=1`: the call was issued with the thread's sets untouched.
    KeptByNegativeControl,
}

impl core::fmt::Display for CapNote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CapNote::Bracket(b) => write!(f, "{b}"),
            CapNote::KeptByNegativeControl => write!(f, "kept(negative-control)"),
        }
    }
}

/// A channel [`born_user`] admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Born {
    /// The channel's handle.
    pub(crate) handle: u32,
    /// What RM stamped, read from the reply.
    pub(crate) privilege: BirthPrivilege,
    /// What the bracket did.
    pub(crate) cap: CapNote,
}

/// Every class host RM constructs as a `KernelChannel`: each `*_CHANNEL_GPFIFO*` class id in
/// kf-abi's generated class table, at every driver tag the table covers. ⊘ Derived, never a list
/// written here: a family added to the table is covered without anyone editing this file.
/// (`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h` maps exactly the GPFIFO channel
/// classes, `GF100_CHANNEL_GPFIFO` through `BLACKWELL_CHANNEL_GPFIFO_B`, to `KernelChannel`.)
fn channel_classes() -> &'static std::collections::BTreeSet<u32> {
    static SET: std::sync::OnceLock<std::collections::BTreeSet<u32>> = std::sync::OnceLock::new();
    SET.get_or_init(|| {
        kf_abi::generated::matrix::ALL_VALUES
            .iter()
            .filter(|v| {
                v.name
                    .strip_prefix("class_ids:")
                    .is_some_and(|n| n.contains("_CHANNEL_GPFIFO"))
            })
            .flat_map(|v| v.runs.iter().filter_map(|r| r.value))
            .filter_map(|id| u32::try_from(id).ok())
            .collect()
    })
}

/// Is `class` one host RM constructs as a channel (and stamps a privilege on)?
#[must_use]
pub fn is_channel_class(class: u32) -> bool {
    channel_classes().contains(&class)
}

/// ★★★ The bound. Called first by every function in this crate that builds an
/// `NV_ESC_RM_ALLOC` request: a channel class is refused, before any host call, unless `birth`
/// proves the request comes from [`born_user`].
///
/// # Errors
/// `CHANNEL_CLASS_OUTSIDE_BIRTH`.
pub(crate) fn admit_alloc_class(
    class: u32,
    birth: Option<&InsideBirthPath>,
) -> Result<(), RmError> {
    if birth.is_none() && is_channel_class(class) {
        eprintln!(
            "kf-host: ⊘ CHANNEL CLASS REFUSED class={class:#06x}: a channel may only be allocated \
             by the birth path (CAP_SYS_ADMIN cleared for the call, RM's reply checked); nothing \
             was sent to host RM"
        );
        return Err(RmError::Other(CHANNEL_CLASS_OUTSIDE_BIRTH));
    }
    Ok(())
}

/// ★★★ Issue one channel-class allocation so that the channel is born `USER`, or refuse it.
///
/// 1. `issue` is called inside the `CAP_SYS_ADMIN` bracket on `ops` (the calling thread, in
///    production) with the [`InsideBirthPath`] token and the request bytes. Unless
///    `skip_bracket` (the negative control), a bracket that cannot clear the bit means `issue`
///    is never called (`CAP_BRACKET_REFUSED`).
/// 2. An allocation error is returned as is; nothing was created.
/// 3. RM's reply in `params` is checked with [`birth_privilege`]. A channel that is not `USER`
///    is passed to `free` and refused (`PRIVILEGED_CHANNEL_REFUSED`).
///
/// # Errors
/// `CAP_BRACKET_REFUSED`, the allocation's own error, or `PRIVILEGED_CHANNEL_REFUSED`.
pub(crate) fn born_user<O: ThreadCapOps + ?Sized>(
    ops: &O,
    skip_bracket: bool,
    engine_type: u32,
    request_flags: u32,
    params: &mut [u8],
    issue: impl FnOnce(&InsideBirthPath, &mut [u8]) -> Result<u32, RmError>,
    free: impl FnOnce(u32),
) -> Result<Born, RmError> {
    let token = InsideBirthPath { _private: () };
    let call = || issue(&token, params);
    let (alloc, cap) = if skip_bracket {
        (call(), CapNote::KeptByNegativeControl)
    } else {
        let (alloc, done) =
            with_effective_cap_cleared_on(ops, CAP_SYS_ADMIN, call).map_err(|refused| {
                eprintln!(
                    "kf-host: ⊘ CHANNEL BIRTH REFUSED engine={engine_type:#x}: CAP_SYS_ADMIN \
                     could not be cleared for the channel-alloc call ({refused}); no channel \
                     was created"
                );
                RmError::Other(CAP_BRACKET_REFUSED)
            })?;
        (alloc, CapNote::Bracket(done))
    };
    let handle = alloc?;
    if let CapNote::Bracket(EffectiveBracket::ClearedNotRestored { errno }) = cap {
        eprintln!(
            "kf-host: CAP_SYS_ADMIN was cleared for a channel birth and could not be restored on \
             this thread (errno {errno}); the thread continues without it in effect"
        );
    }
    // ★★★ THE REPLY CHECK: RM's verdict, read from the reply, whatever the bracket did.
    match birth_privilege(request_flags, params) {
        Ok(privilege) => Ok(Born {
            handle,
            privilege,
            cap,
        }),
        Err(why) => {
            eprintln!(
                "kf-host: ⊘ PRIVILEGED CHANNEL REFUSED h={handle:#x} engine={engine_type:#x}: \
                 {why} (cap_sys_admin={cap}); the channel is freed. Guest-authored work must \
                 never run on an ADMIN or KERNEL host channel."
            );
            free(handle);
            Err(RmError::Other(PRIVILEGED_CHANNEL_REFUSED))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE, RingSpec, channel_alloc_request};
    use kf_abi::submit::ChannelAllocParams;
    use kf_linux_raw::capability::{ThisThread, ThreadCaps};
    use std::cell::Cell;

    const ROOT_CAPS: ThreadCaps = ThreadCaps {
        effective: 0x0000_01ff_ffff_ffff,
        permitted: 0x0000_01ff_ffff_ffff,
        inheritable: 0,
    };

    /// A simulated thread that holds `CAP_SYS_ADMIN` (or not), with `capset` refusable.
    struct FakeThread {
        caps: Cell<ThreadCaps>,
        refuse_clear: bool,
    }

    impl FakeThread {
        fn root() -> Self {
            FakeThread {
                caps: Cell::new(ROOT_CAPS),
                refuse_clear: false,
            }
        }
        fn holds(&self) -> bool {
            self.caps.get().holds_effective(CAP_SYS_ADMIN)
        }
    }

    impl ThreadCapOps for FakeThread {
        fn get(&self) -> Result<ThreadCaps, i32> {
            Ok(self.caps.get())
        }
        fn set(&self, caps: ThreadCaps) -> Result<(), i32> {
            if self.refuse_clear && !caps.holds_effective(CAP_SYS_ADMIN) {
                return Err(1);
            }
            self.caps.set(caps);
            Ok(())
        }
    }

    fn request() -> (u32, [u8; ChannelAllocParams::SIZE]) {
        let ring = RingSpec {
            gp_fifo_va: 0x1_0000_0000,
            gp_fifo_entries: 1024,
            userd_memory: 0xcafe_0010,
            userd_offset: 0,
            err_notifier: 0,
        };
        let req = channel_alloc_request(&ring, 0x1);
        let mut b = [0u8; ChannelAllocParams::SIZE];
        req.encode_into(&mut b).expect("encode");
        (req.flags, b)
    }

    /// RM's reply: the request with `flags` (+20) replaced.
    fn answer(p: &mut [u8], flags: u32) {
        p[20..24].copy_from_slice(&flags.to_le_bytes());
    }

    /// ★★★ The alloc runs INSIDE the bracket on a thread that holds the capability, and the
    /// birth is admitted only after the reply check. A `born_user` that issued the call outside
    /// the bracket turns this red on any runner, root or not.
    #[test]
    fn the_alloc_runs_without_cap_sys_admin_on_a_root_thread() {
        let t = FakeThread::root();
        let (flags, mut p) = request();
        let held_at_issue = Cell::new(None);
        let freed = Cell::new(None);
        let born = born_user(
            &t,
            false,
            0x1,
            flags,
            &mut p,
            |_inside, p| {
                held_at_issue.set(Some(t.holds()));
                answer(p, 0x80);
                Ok(0xcafe_0020)
            },
            |h| freed.set(Some(h)),
        )
        .expect("a USER reply is admitted");
        assert_eq!(
            held_at_issue.get(),
            Some(false),
            "issued with CAP_SYS_ADMIN"
        );
        assert_eq!(born.handle, 0xcafe_0020);
        assert_eq!(born.privilege.reply_flags, 0x80);
        assert_eq!(
            born.cap,
            CapNote::Bracket(EffectiveBracket::ClearedAndRestored)
        );
        assert!(
            t.holds(),
            "the thread's set was not restored after the call"
        );
        assert_eq!(freed.get(), None);
    }

    /// ★★★ RM's verdict refuses the birth and frees the channel, even though the alloc itself
    /// succeeded. A `born_user` whose reply check is replaced by `Ok` turns this red.
    #[test]
    fn a_privileged_reply_is_refused_and_the_channel_freed() {
        for reply in [0x20u32, 0xa0, 0x0040_00a0] {
            let t = FakeThread::root();
            let (flags, mut p) = request();
            let freed = Cell::new(None);
            let r = born_user(
                &t,
                false,
                0x1,
                flags,
                &mut p,
                |_inside, p| {
                    answer(p, reply);
                    Ok(0xcafe_0021)
                },
                |h| freed.set(Some(h)),
            );
            assert_eq!(
                r,
                Err(RmError::Other(PRIVILEGED_CHANNEL_REFUSED)),
                "{reply:#x}"
            );
            assert_eq!(freed.get(), Some(0xcafe_0021), "{reply:#x}: not freed");
        }
    }

    /// Fail closed: a bracket that cannot clear the bit never issues the alloc.
    #[test]
    fn a_bracket_that_cannot_clear_issues_nothing() {
        let t = FakeThread {
            caps: Cell::new(ROOT_CAPS),
            refuse_clear: true,
        };
        let (flags, mut p) = request();
        let issued = Cell::new(false);
        let r = born_user(
            &t,
            false,
            0x1,
            flags,
            &mut p,
            |_inside, _p| {
                issued.set(true);
                Ok(1)
            },
            |_| {},
        );
        assert_eq!(r, Err(RmError::Other(CAP_BRACKET_REFUSED)));
        assert!(
            !issued.get(),
            "the alloc was issued with CAP_SYS_ADMIN held"
        );
    }

    /// An alloc that fails creates nothing: its error is returned and nothing is freed.
    #[test]
    fn an_alloc_error_is_returned_and_frees_nothing() {
        let t = FakeThread::root();
        let (flags, mut p) = request();
        let freed = Cell::new(false);
        let r = born_user(
            &t,
            false,
            0x1,
            flags,
            &mut p,
            |_inside, _p| Err(RmError::NoMemory),
            |_| freed.set(true),
        );
        assert_eq!(r, Err(RmError::NoMemory));
        assert!(!freed.get());
        assert!(t.holds());
    }

    /// The negative control skips the bracket (the alloc sees the bit held), and the reply check
    /// still refuses what RM then stamps.
    #[test]
    fn the_negative_control_skips_the_bracket_and_the_reply_check_still_refuses() {
        let t = FakeThread::root();
        let (flags, mut p) = request();
        let held = Cell::new(None);
        let freed = Cell::new(None);
        let r = born_user(
            &t,
            true,
            0x1,
            flags,
            &mut p,
            |_inside, p| {
                held.set(Some(t.holds()));
                answer(p, 0xa0);
                Ok(7)
            },
            |h| freed.set(Some(h)),
        );
        assert_eq!(held.get(), Some(true));
        assert_eq!(r, Err(RmError::Other(PRIVILEGED_CHANNEL_REFUSED)));
        assert_eq!(freed.get(), Some(7));
    }

    /// ★ A request that asks for bit 5 is refused: the readback would only echo it.
    #[test]
    fn a_request_asking_for_privilege_is_refused() {
        let t = FakeThread::root();
        let (flags, mut p) = request();
        let freed = Cell::new(None);
        let r = born_user(
            &t,
            false,
            0x1,
            flags | NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE,
            &mut p,
            |_inside, p| {
                answer(p, 0x80);
                Ok(9)
            },
            |h| freed.set(Some(h)),
        );
        assert_eq!(r, Err(RmError::Other(PRIVILEGED_CHANNEL_REFUSED)));
        assert_eq!(freed.get(), Some(9));
    }

    /// ★★★ KNOWN-POSITIVE for the bound: the bench's own channel class (GA106's
    /// `AMPERE_CHANNEL_GPFIFO_A`) and every derived channel class are refused without the token,
    /// and admitted with it. The objects kf-host allocates every day are admitted without one.
    #[test]
    fn a_channel_class_is_refused_outside_the_birth_path() {
        use kf_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A;
        assert_eq!(
            admit_alloc_class(AMPERE_CHANNEL_GPFIFO_A, None),
            Err(RmError::Other(CHANNEL_CLASS_OUTSIDE_BIRTH))
        );
        let token = InsideBirthPath { _private: () };
        for &c in channel_classes() {
            assert_eq!(
                admit_alloc_class(c, None),
                Err(RmError::Other(CHANNEL_CLASS_OUTSIDE_BIRTH)),
                "{c:#06x}"
            );
            assert_eq!(admit_alloc_class(c, Some(&token)), Ok(()), "{c:#06x}");
        }
        for c in [
            kf_abi::invariant_classes::CHANNEL_GROUP,
            kf_abi::invariant_classes::VA_SPACE,
            kf_abi::bringup::NV01_MEMORY_VIRTUAL,
            0x0000_0041, // NV01_ROOT_CLIENT
            0x0000_9096, // GF100_ZBC_CLEAR (kf-qemu rmfacts)
        ] {
            assert!(!is_channel_class(c), "{c:#06x}");
            assert_eq!(admit_alloc_class(c, None), Ok(()), "{c:#06x}");
        }
    }

    /// The derived universe is not empty and holds the channel class of every generation from
    /// Fermi to Blackwell (11 in `resource_list.h` at 580). Floor, not an exact count: the table
    /// grows when a driver tag adds a class.
    #[test]
    fn the_channel_class_universe_is_derived_and_complete() {
        let set = channel_classes();
        assert!(
            set.len() >= 11,
            "only {} channel classes derived: {set:x?}",
            set.len()
        );
        for c in [
            0x906f, 0xa06f, 0xa16f, 0xb06f, 0xc06f, 0xc36f, 0xc46f, 0xc56f, 0xc86f, 0xc96f, 0xca6f,
        ] {
            assert!(set.contains(&c), "{c:#06x} missing from {set:x?}");
        }
        // Nothing that is not a channel crept in through the name filter.
        for &c in set {
            assert_eq!(c & 0xff, 0x6f, "{c:#06x} is not a GPFIFO channel class id");
        }
    }

    /// ★ LIVE, on the real thread: the issuing closure reads its own `CapEff` from procfs, and
    /// `CAP_SYS_ADMIN` must be absent. As root this is the production bracket end to end; an
    /// unprivileged run says `CAP-GATE: VACUOUS`.
    #[test]
    fn live_cap_born_user_issues_without_cap_sys_admin() {
        let held = kf_linux_raw::capability::current_thread_caps()
            .expect("capget")
            .holds_effective(CAP_SYS_ADMIN);
        kf_linux_raw::capability::report_live(
            "kf-host live_cap_born_user_issues_without_cap_sys_admin",
            held,
        );
        let (flags, mut p) = request();
        let eff = Cell::new(u64::MAX);
        let born = born_user(
            &ThisThread,
            false,
            0x1,
            flags,
            &mut p,
            |_inside, p| {
                let s = std::fs::read_to_string("/proc/thread-self/status").expect("procfs");
                let v = s
                    .lines()
                    .find_map(|l| l.strip_prefix("CapEff:"))
                    .map(|x| u64::from_str_radix(x.trim(), 16).expect("hex"))
                    .expect("CapEff");
                eff.set(v);
                answer(p, 0x80);
                Ok(1)
            },
            |_| {},
        )
        .expect("born");
        assert_eq!(
            eff.get() & (1 << CAP_SYS_ADMIN),
            0,
            "issued with CAP_SYS_ADMIN"
        );
        let after = kf_linux_raw::capability::current_thread_caps().expect("capget");
        assert_eq!(after.holds_effective(CAP_SYS_ADMIN), held, "not restored");
        assert_eq!(
            born.cap,
            CapNote::Bracket(if held {
                EffectiveBracket::ClearedAndRestored
            } else {
                EffectiveBracket::NotHeld
            })
        );
    }
}
