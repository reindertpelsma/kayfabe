// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **No CUDA call runs on a thread that holds `CAP_SYS_ADMIN` in effect** — so every channel
//! libcuda allocates is born `USER` (`docs/design/THE_CONSTRAINTS.md` §30; OWNER_RULINGS §P).
//!
//! Host RM stamps a channel's privilege at the channel-alloc ioctl from the calling thread's
//! `capable(CAP_SYS_ADMIN)` (`ogkm-580: kernel_channel.c:277-291`, `escape.c:304`). kf-host
//! brackets its own channel births one call at a time (`kf_host::birth`). libcuda's are out of
//! reach of a bracket: it allocates its context's channels inside `cuCtxCreate`, may allocate
//! more later on whatever thread makes a call, and runs threads of its own. So the bit is
//! cleared from the **thread's** effective set for the rest of its life, before the thread's
//! first CUDA call ([`kf_linux_raw::capability::clear_effective_for_thread_life`]). Threads the
//! thread starts afterwards, libcuda's included, copy its sets and start without the bit. The
//! permitted set is untouched, and no other thread changes.
//!
//! [`cuda_thread`] runs at every entry by which a thread starts CUDA work: loading the library
//! (`Cuda::open_soname`, before `dlopen`), `cuInit`, `cuCtxCreate` and `cuCtxSetCurrent` (a call
//! on a thread with no current context fails). It costs one thread-local read after the first
//! call on a thread. If the bit is held and cannot be cleared, the entry refuses by name and
//! no CUDA call is made.
//!
//! ⚠ The calling thread keeps the bit cleared after the call returns. A VMM must therefore make
//! its CUDA calls on threads of their own (kf3 does: `kf_qemu::device::on_cuda_thread` for
//! realize, the VA-manager and display threads after), never on a thread it needs
//! `CAP_SYS_ADMIN` on later.

use crate::driver_unsafe::CudaError;
use kf_linux_raw::capability::{
    BracketRefused, CAP_SYS_ADMIN, ThreadCapOps, ThreadLifeClear,
    clear_effective_for_thread_life_on,
};
use std::cell::Cell;

thread_local! {
    /// This thread already passed [`cuda_thread`].
    static DONE: Cell<bool> = const { Cell::new(false) };
}

/// The calling thread's name, for the log line.
fn thread_name() -> String {
    std::thread::current().name().map_or_else(
        || format!("{:?}", std::thread::current().id()),
        str::to_string,
    )
}

/// ★ Before the calling thread's first CUDA call: clear `CAP_SYS_ADMIN` from its effective set
/// for the rest of its life (see the module docs). One log line per thread, the first time.
///
/// # Errors
/// [`CudaError::Refused`] (`what` = `"CAP_SYS_ADMIN clear"`) when the bit is held and could not be
/// cleared; the caller must not make the CUDA call.
pub fn cuda_thread() -> Result<(), CudaError> {
    DONE.with(|done| posture_memo_on(&kf_linux_raw::capability::ThisThread, done, &thread_name))
}

/// ★ [`cuda_thread`] over any [`ThreadCapOps`] and memo cell (a simulated thread in tests).
///
/// ⊘⊘ 2026-11-14 (security review 2 D-13), above the memo it corrects: the memo used to
/// short-circuit for the rest of the thread's life after the first clear. The permitted set keeps
/// `CAP_SYS_ADMIN`, so code on the thread could raise it again, and every later CUDA entry would
/// then create ADMIN channels with no refusal. Now a memoised thread is RE-VERIFIED on every entry
/// (one `capget`, no log line): the bit back in effect is cleared again (logged, as the first
/// time), or the entry refuses by name. A failed read is treated as "held" (fail closed).
///
/// # Errors
/// As [`cuda_thread`].
pub fn posture_memo_on<O: ThreadCapOps + ?Sized>(
    ops: &O,
    done: &Cell<bool>,
    thread: &dyn Fn() -> String,
) -> Result<(), CudaError> {
    if done.get() {
        if ops
            .get()
            .is_ok_and(|caps| !caps.holds_effective(CAP_SYS_ADMIN))
        {
            return Ok(());
        }
        done.set(false);
    }
    let r = posture_on(ops, &thread());
    if r.is_ok() {
        done.set(true);
    }
    r.map(|_| ())
}

/// [`cuda_thread`]'s decision over any [`ThreadCapOps`] (a simulated thread in tests), without
/// the per-thread memo.
///
/// # Errors
/// As [`cuda_thread`].
pub fn posture_on<O: ThreadCapOps + ?Sized>(
    ops: &O,
    thread: &str,
) -> Result<ThreadLifeClear, CudaError> {
    match clear_effective_for_thread_life_on(ops, CAP_SYS_ADMIN) {
        Ok(what) => {
            eprintln!(
                "kf-cuda: cuda thread posture thread={thread} cap_sys_admin={what} (libcuda's \
                 channels from this thread, and from threads it starts, are created without \
                 CAP_SYS_ADMIN)"
            );
            Ok(what)
        }
        Err(refused) => {
            eprintln!(
                "kf-cuda: ⊘ CUDA THREAD REFUSED thread={thread}: CAP_SYS_ADMIN is in the \
                 effective set and could not be cleared ({refused}); no CUDA call is made on \
                 this thread"
            );
            Err(refusal(refused))
        }
    }
}

fn refusal(r: BracketRefused) -> CudaError {
    CudaError::Refused {
        what: "CAP_SYS_ADMIN clear",
        code: 0,
        name: format!(
            "{r}: a CUDA call on a thread holding CAP_SYS_ADMIN would create ADMIN channels"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_linux_raw::capability::ThreadCaps;

    const ROOT_CAPS: ThreadCaps = ThreadCaps {
        effective: 0x0000_01ff_ffff_ffff,
        permitted: 0x0000_01ff_ffff_ffff,
        inheritable: 0,
    };

    struct FakeThread {
        caps: Cell<ThreadCaps>,
        refuse: bool,
        sets: Cell<u32>,
    }

    impl ThreadCapOps for FakeThread {
        fn get(&self) -> Result<ThreadCaps, i32> {
            Ok(self.caps.get())
        }
        fn set(&self, caps: ThreadCaps) -> Result<(), i32> {
            self.sets.set(self.sets.get() + 1);
            if self.refuse {
                return Err(1);
            }
            self.caps.set(caps);
            Ok(())
        }
    }

    fn root(refuse: bool) -> FakeThread {
        FakeThread {
            caps: Cell::new(ROOT_CAPS),
            refuse,
            sets: Cell::new(0),
        }
    }

    /// ★★ On a simulated root thread the bit is cleared and stays cleared; permitted is kept.
    #[test]
    fn a_root_thread_is_cleared_for_its_life() {
        let t = root(false);
        assert_eq!(posture_on(&t, "t"), Ok(ThreadLifeClear::Cleared));
        assert!(!t.caps.get().holds_effective(CAP_SYS_ADMIN));
        assert_eq!(t.caps.get().permitted, ROOT_CAPS.permitted);
        assert_eq!(posture_on(&t, "t"), Ok(ThreadLifeClear::NotHeld));
        assert_eq!(t.sets.get(), 1, "cleared once, never restored");
    }

    /// ★★ D-13 (2026-11-14): a memoised thread whose `CAP_SYS_ADMIN` was RAISED again (it stays
    /// permitted) is cleared again on its next CUDA entry — the memo no longer vouches for it. A
    /// quiet entry while it is clear costs one read and no write.
    #[test]
    fn d13_a_bit_raised_again_after_the_memo_is_cleared_again() {
        let t = root(false);
        let done = Cell::new(false);
        let name = || "t".to_string();
        posture_memo_on(&t, &done, &name).unwrap();
        assert!(done.get());
        assert_eq!(t.sets.get(), 1);
        posture_memo_on(&t, &done, &name).unwrap();
        assert_eq!(t.sets.get(), 1, "clear and memoised: no write");
        // safe code on the thread raises it again (it is still permitted)
        t.caps.set(ROOT_CAPS);
        posture_memo_on(&t, &done, &name).unwrap();
        assert!(
            !t.caps.get().holds_effective(CAP_SYS_ADMIN),
            "cleared again"
        );
        assert_eq!(t.sets.get(), 2);
        assert!(done.get());
        // fail closed: a memoised thread whose sets cannot be read is not vouched for
        struct Unreadable;
        impl ThreadCapOps for Unreadable {
            fn get(&self) -> Result<ThreadCaps, i32> {
                Err(13)
            }
            fn set(&self, _: ThreadCaps) -> Result<(), i32> {
                Err(13)
            }
        }
        assert!(posture_memo_on(&Unreadable, &done, &name).is_err());
        assert!(!done.get());
    }

    /// ★★ D-13: a re-raised bit that can no longer be cleared refuses the entry, and the memo is
    /// dropped, so no later entry passes on it either.
    #[test]
    fn d13_a_re_raised_bit_that_cannot_be_cleared_refuses_every_later_entry() {
        let t = root(false);
        let done = Cell::new(false);
        let name = || "t".to_string();
        posture_memo_on(&t, &done, &name).unwrap();
        let stuck = FakeThread {
            caps: Cell::new(ROOT_CAPS),
            refuse: true,
            sets: Cell::new(0),
        };
        for _ in 0..2 {
            assert!(matches!(
                posture_memo_on(&stuck, &done, &name),
                Err(CudaError::Refused {
                    what: "CAP_SYS_ADMIN clear",
                    ..
                })
            ));
            assert!(!done.get());
        }
        // a thread whose sets cannot even be read is treated as holding the bit
        struct Unreadable;
        impl ThreadCapOps for Unreadable {
            fn get(&self) -> Result<ThreadCaps, i32> {
                Err(1)
            }
            fn set(&self, _: ThreadCaps) -> Result<(), i32> {
                Err(1)
            }
        }
        let done = Cell::new(true);
        assert!(posture_memo_on(&Unreadable, &done, &name).is_err());
        assert!(!done.get());
    }

    /// ★ Fail closed: a held bit that cannot be cleared refuses the CUDA entry by name.
    #[test]
    fn a_bit_that_cannot_be_cleared_refuses_the_cuda_call() {
        let t = root(true);
        match posture_on(&t, "t") {
            Err(CudaError::Refused { what, .. }) => assert_eq!(what, "CAP_SYS_ADMIN clear"),
            other => panic!("expected a named refusal, got {other:?}"),
        }
        assert!(t.caps.get().holds_effective(CAP_SYS_ADMIN));
    }

    /// ★ LIVE, through the real entry: `Cuda::open_soname` clears the bit before it tries to load
    /// anything, so even a library that is not there leaves the thread without it. A binding
    /// that loaded libcuda first would fail this as root (CI runs it as root).
    #[test]
    fn live_cap_open_clears_before_dlopen() {
        let held = kf_linux_raw::capability::current_thread_caps()
            .expect("capget")
            .holds_effective(CAP_SYS_ADMIN);
        kf_linux_raw::capability::report_live("kf-cuda live_cap_open_clears_before_dlopen", held);
        let (r, eff) = std::thread::spawn(|| {
            let r =
                crate::driver_unsafe::Cuda::open_soname("libkf-no-such-library.so.0").map(|_| ());
            let caps = kf_linux_raw::capability::current_thread_caps().expect("capget");
            (r, caps)
        })
        .join()
        .expect("join");
        assert!(
            matches!(r, Err(CudaError::NoLibrary { .. })),
            "expected the load to fail by name, got {r:?}"
        );
        assert!(
            !eff.holds_effective(CAP_SYS_ADMIN),
            "open_soname reached dlopen with CAP_SYS_ADMIN in effect"
        );
    }

    /// ★ LIVE: on a thread of its own, `cuda_thread` leaves `CAP_SYS_ADMIN` out of the effective
    /// set (read from procfs), a thread started afterwards inherits that, and the test harness's
    /// thread is untouched. Unprivileged it prints `CAP-GATE: VACUOUS`.
    #[test]
    fn live_cap_cuda_thread_clears_before_the_first_call() {
        let outer = kf_linux_raw::capability::current_thread_caps().expect("capget");
        kf_linux_raw::capability::report_live(
            "kf-cuda live_cap_cuda_thread_clears_before_the_first_call",
            outer.holds_effective(CAP_SYS_ADMIN),
        );
        let (eff, child) = std::thread::Builder::new()
            .name("kf-cuda-posture-test".into())
            .spawn(|| {
                cuda_thread().expect("posture");
                cuda_thread().expect("memoised");
                let s = std::fs::read_to_string("/proc/thread-self/status").expect("procfs");
                let eff = s
                    .lines()
                    .find_map(|l| l.strip_prefix("CapEff:"))
                    .map(|x| u64::from_str_radix(x.trim(), 16).expect("hex"))
                    .expect("CapEff");
                let child = std::thread::spawn(|| {
                    kf_linux_raw::capability::current_thread_caps().expect("capget")
                })
                .join()
                .expect("join");
                (eff, child)
            })
            .expect("spawn")
            .join()
            .expect("join");
        assert_eq!(
            eff & (1 << CAP_SYS_ADMIN),
            0,
            "CAP_SYS_ADMIN still in effect"
        );
        assert!(
            !child.holds_effective(CAP_SYS_ADMIN),
            "a later thread inherited it"
        );
        assert_eq!(
            kf_linux_raw::capability::current_thread_caps().expect("capget"),
            outer,
            "the posture reached another thread"
        );
    }
}
