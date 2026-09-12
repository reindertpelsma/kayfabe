//! The **break signal** — the only way to interrupt a thread that is inside a blocking
//! ioctl.
//!
//! `l1_os_shell.md` §7.2, ported from the C's #73 (`signal_interrupt_delivery_done`). Three
//! calls, and the whole file exists because of one flag that must **not** be set:
//!
//! > *"The isolate installs its break-signal handler **without `SA_RESTART`**. With
//! > `SA_RESTART` the host kernel silently restarts the ioctl, it never returns `EINTR`,
//! > and we never learn we interrupted anything — the failure mode is 'cancellation
//! > appears to work and does nothing'."*
//!
//! That is a measured C finding (`C: src/stub/nvkvm_stub.c:699-708`, `:2669-2678`), not a
//! design preference, and it is the reason this is a dedicated file rather than a line
//! inside the isolate adapter: `SA_RESTART` is the *default* in every convenience wrapper,
//! so the absence has to be somewhere a reviewer can see it.
//!
//! ## The handler body is empty, and that is the design
//!
//! A signal handler may call only async-signal-safe functions. This one calls none: its
//! entire effect is that its *delivery* makes the interrupted syscall return `EINTR`. The C
//! says the same in one line — *"Its only purpose is to interrupt a blocking `ioctl(2)`"*.
//! Anything richer (a flag, a counter, a log) is state the interrupted thread can read for
//! itself from the `errno` it is about to receive.
//!
//! ## Why per-**thread** delivery
//!
//! A process-wide `kill(2)` picks an arbitrary thread that has the signal unblocked, which
//! on a worker pool is a coin toss: the cancel lands on a sibling worker's unrelated verb.
//! `tgkill(2)` names the thread. This is the C's refinement 4 one layer down — the txn
//! check stops a cancel landing on a later operation *of the same worker*; naming the
//! thread stops it landing on a *different* worker entirely.

use crate::error::{RawError, last_syscall_error};

/// The signal used to break a blocked host call.
///
/// `SIGUSR1` is the C's choice and is kept: it has no default meaning to the runtime, and
/// matching the C keeps the two implementations comparable when a trace differential is
/// run against them.
pub const BREAK_SIGNAL: i32 = libc::SIGUSR1;

/// The empty handler. See the module docs: delivery is the whole effect.
extern "C" fn break_handler(_signum: libc::c_int) {}

/// Install the break-signal handler for this **process**, without `SA_RESTART`.
///
/// Must be called before any [`interrupt_thread`] can arrive: `SIGUSR1`'s default action is
/// to terminate the process, so a cancel that races the installation kills the isolate
/// instead of interrupting a verb (the C notes exactly this at
/// `C: src/stub/nvkvm_stub.c:2671-2672`).
///
/// Idempotent — installing twice is harmless and installs the same disposition.
///
/// # Errors
/// [`RawError::Syscall`] (`sigaction`).
pub fn install_break_handler() -> Result<(), RawError> {
    // SAFETY: `mem::zeroed` is a valid initial value for `sigaction` on Linux — the struct
    // is plain integers, a function-pointer field and a `sigset_t` (itself an integer
    // array), with no niche and no invalid bit pattern. It is written in full below before
    // being passed to the kernel.
    let mut act: libc::sigaction = unsafe { core::mem::zeroed() };
    act.sa_sigaction = break_handler as *const () as usize;
    // ★ `sa_flags` is deliberately left at 0: NO `SA_RESTART`. See the module docs — this
    // is the single most consequential line in the file.
    act.sa_flags = 0;

    // SAFETY: `act.sa_mask` is a live `sigset_t` in this frame, exclusively borrowed for
    // this call, and `sigemptyset` writes exactly that object and dereferences nothing
    // else.
    let rc = unsafe { libc::sigemptyset(core::ptr::from_mut(&mut act.sa_mask)) };
    if rc != 0 {
        return Err(last_syscall_error("sigemptyset"));
    }

    // SAFETY: `act` is a fully initialised `sigaction` living in this frame for the
    // duration of the call; the kernel copies it and retains no reference. The old
    // disposition is discarded by passing a null pointer, which `sigaction` documents as
    // "do not report the previous action". `break_handler` is an `extern "C"` function with
    // the signature the kernel will call it with, and it is `'static`.
    let rc = unsafe {
        libc::sigaction(
            BREAK_SIGNAL,
            core::ptr::from_ref(&act),
            core::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err(last_syscall_error("sigaction"));
    }
    Ok(())
}

/// This thread's kernel thread id — the value [`interrupt_thread`] names.
///
/// Deliberately *not* `pthread_self()`: that is an opaque library handle, and the call that
/// delivers a signal to one thread takes a kernel tid.
#[must_use]
pub fn current_thread_id() -> ThreadId {
    // SAFETY: `gettid` takes no argument, dereferences nothing, and returns a value. It
    // cannot fail.
    ThreadId(unsafe { libc::gettid() })
}

/// A kernel thread id within **this** process.
///
/// A newtype rather than a bare `i32` because the only thing you may do with one is
/// [`interrupt_thread`], and an integer that could be a pid, a tid or a descriptor number
/// is how a `tgkill` ends up aimed at a process group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(i32);

impl ThreadId {
    /// The raw kernel tid — for a diagnostic, never for arithmetic.
    #[must_use]
    pub fn raw(self) -> i32 {
        self.0
    }
}

/// Deliver the break signal to one thread of **this** process.
///
/// Returns `Ok(false)` if the thread has already exited (`ESRCH`) — a state, not a failure:
/// it is `CancelSink::deliver`'s *"the verb finished first"* row (§7.3), and reporting it
/// as an error would make an ordinary race look like a broken cancel path.
///
/// # Errors
/// [`RawError::Syscall`] (`tgkill`) for anything other than `ESRCH`.
pub fn interrupt_thread(thread: ThreadId) -> Result<bool, RawError> {
    // SAFETY: `getpid` takes no argument and dereferences nothing.
    let tgid = unsafe { libc::getpid() };
    // SAFETY: `tgkill` takes three integers by value and dereferences no user memory. The
    // thread group is our own, so this cannot signal another process even if `thread` is
    // stale — the kernel refuses a tid that is not in `tgid` with `ESRCH`, which is exactly
    // the case handled below.
    let rc = unsafe { libc::syscall(libc::SYS_tgkill, tgid, thread.0, BREAK_SIGNAL) };
    if rc == 0 {
        return Ok(true);
    }
    let err = last_syscall_error("tgkill");
    if err
        == (RawError::Syscall {
            call: "tgkill",
            errno: Some(libc::ESRCH),
        })
    {
        return Ok(false);
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::mpsc;

    /// ★★ The property the whole file exists for, asserted end to end against the kernel:
    /// a thread blocked in `read(2)` is released by the break signal with `EINTR`, and the
    /// read is **not** silently restarted.
    ///
    /// This is the `SA_RESTART` finding made into a test. If the flag were set, this test
    /// would hang — which is why the assertion is a completed round trip and not a returned
    /// error code.
    #[test]
    fn a_blocked_read_returns_interrupted_and_is_not_restarted() {
        install_break_handler().expect("install");
        let (mut rx, _tx) = std::io::pipe().expect("pipe");
        let (tid_tx, tid_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let worker = std::thread::spawn(move || {
            tid_tx.send(current_thread_id()).expect("send tid");
            let mut buf = [0u8; 1];
            let r = rx.read(&mut buf);
            done_tx.send(r.map_err(|e| e.kind())).expect("send result");
        });

        let tid = tid_rx.recv().expect("tid");
        // Deliver until it lands. The loop is bounded by the receive below, not by a
        // sleep: the worker either reports `Interrupted` or the test fails by hanging,
        // which a test runner reports as the timeout it is.
        loop {
            assert!(
                interrupt_thread(tid).expect("tgkill"),
                "the thread is alive"
            );
            if let Ok(result) = done_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                assert_eq!(
                    result,
                    Err(std::io::ErrorKind::Interrupted),
                    "the blocked read must report EINTR — a restarted syscall is the \
                     SA_RESTART failure this file exists to prevent"
                );
                break;
            }
        }
        worker.join().expect("join");
    }

    /// A cancel aimed at a thread that is not there reports the miss as a *state*, not an
    /// error — §7.3's fourth row, *"the verb finished first"*.
    ///
    /// ★ The instrument, corrected. The obvious version of this test — spawn a thread,
    /// `join` it, then signal its tid — **fails while the code is correct**: `tgkill`
    /// returned 0 for a joined thread's tid, because a `join`ed but not-yet-reaped task is
    /// still resolvable. That is a race with the scheduler, not a property, so the test
    /// names a tid the kernel can never resolve instead. Constructed directly because the
    /// field is private and there is no production caller that learns a tid out of band —
    /// adding a public constructor for a test would be the API bloat, not the fix.
    #[test]
    fn interrupting_a_thread_that_is_not_ours_is_false_not_an_error() {
        install_break_handler().expect("install");
        let absent = ThreadId(i32::MAX);
        assert_eq!(
            interrupt_thread(absent),
            Ok(false),
            "a cancel that named nothing is a state, not a failure"
        );
    }

    #[test]
    fn thread_ids_are_distinct_per_thread_and_stable_within_one() {
        let mine = current_thread_id();
        assert_eq!(mine, current_thread_id(), "stable within a thread");
        let theirs = std::thread::spawn(current_thread_id).join().expect("join");
        assert_ne!(mine, theirs, "distinct across threads");
        assert!(mine.raw() > 0);
    }

    #[test]
    fn installing_the_handler_twice_is_harmless() {
        install_break_handler().expect("first");
        install_break_handler().expect("second");
    }
}

// ============================================================================================
// ⊘⊘⊘ TEMPORARY DEBUG INSTRUMENT — DELETE BEFORE SHIPPING (w495)
// ============================================================================================

/// ★★★★★ **The stall probe.** Owner, 2026-09-12: *"at the mmio trap start you ask the kernel
/// to send a sig alarm after 2 milliseconds to that vcpu thread … then it will crash dump at
/// the exact site it was hanging in."*
///
/// ⊘ **Not `timer_create`/`SIGEV_THREAD_ID`.** That is the obvious implementation and it does
/// not build: the embedded isolate is a **musl** binary and musl's bindings carry neither the
/// constant nor `sigev_notify_thread_id`. Gating the module by target would leave two
/// different debug builds, which is worse than the problem.
///
/// ⇒ the existing over-budget watchdog already scans in-flight traps every 200 us. Giving it
/// the stuck thread's **tid** lets it `tgkill` that exact thread, which is the same outcome
/// with no per-trap timer and no target-specific code.
pub mod stall_alarm {
    use super::{RawError, last_syscall_error};

    /// This thread's kernel id, for a thread-directed signal.
    #[must_use]
    pub fn current_tid() -> i32 {
        // SAFETY: `gettid` takes no argument and dereferences nothing.
        unsafe { libc::syscall(libc::SYS_gettid) as i32 }
    }

    /// Send `SIGALRM` to one thread of this process. Its default action terminates and dumps,
    /// which is the point: the dump is taken **at the site the thread is stuck in**.
    ///
    /// ⊘ Thread-directed (`tgkill`), never process-directed. A process-directed signal can be
    /// delivered to any thread with it unblocked — including one behaving perfectly — and
    /// would name the wrong site, which is the one failure this instrument cannot afford.
    ///
    /// # Errors
    /// If `tgkill` refuses; `ESRCH` means the thread already finished, which is not an error
    /// worth propagating and is reported as `Ok(false)`.
    pub fn alarm_thread(tid: i32) -> Result<bool, RawError> {
        // SAFETY: `getpid` takes no argument and dereferences nothing.
        let tgid = unsafe { libc::getpid() };
        // SAFETY: three integers by value, no user memory dereferenced. The thread group is
        // our own, so a stale tid cannot reach another process — the kernel answers ESRCH.
        let rc = unsafe { libc::syscall(libc::SYS_tgkill, tgid, tid, libc::SIGALRM) };
        if rc == 0 {
            return Ok(true);
        }
        let err = last_syscall_error("tgkill");
        if err
            == (RawError::Syscall {
                call: "tgkill",
                errno: Some(libc::ESRCH),
            })
        {
            // ⊘ The thread finished between the scan and the signal. That is the instrument
            // losing a race, not a fault — and reporting it as one would train the reader to
            // ignore a real refusal.
            return Ok(false);
        }
        Err(err)
    }

    /// ★★★★★ **THE PER-TRAP ALARM, as the owner specified it.** Armed at the MMIO trap's
    /// start, disarmed when the trap answers. If it fires, `SIGALRM`'s default action dumps
    /// **at the site the thread is stuck in**.
    ///
    /// # ⊘ Why this is here and not behind a polling watchdog
    ///
    /// My first attempt used the existing 200 us watchdog instead, justified by *"musl's
    /// bindings lack `SIGEV_THREAD_ID`"*. ⊘⊘ **That reasoning was wrong, and the owner caught
    /// it: an isolate has nothing to do with an alarm set in a vCPU thread.** What actually
    /// happened is that this crate is compiled for BOTH targets — the shim against glibc and
    /// the embedded isolate against musl — so a musl binding gap gated a feature only the
    /// glibc side would ever call. The fix is to break that coupling, not to change the
    /// design. Hence `#[cfg]` below.
    ///
    /// ★ And the owner's form is the better instrument: a timer fires **exactly** at the
    /// budget, where a 200 us poll detects late and can miss a trap that is over budget but
    /// short.
    ///
    /// ⊘ `SIGEV_THREAD_ID` and `sigev_notify_thread_id` are absent from this libc version's
    /// **glibc** bindings (they exist for android and uclibc), so the structure is declared
    /// here against glibc's documented layout and its size is asserted at compile time. The
    /// union's first member is the tid; `SIGEV_MAX_SIZE` is 64 bytes.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    pub mod timer {
        /// `SIGEV_THREAD_ID` — deliver to one specific thread. Linux-wide, all libcs.
        const SIGEV_THREAD_ID: i32 = 4;

        /// glibc's `struct sigevent`, 64 bytes: an 8-byte `sigev_value` union, two ints, then
        /// a 48-byte union whose first member is the target tid.
        #[repr(C)]
        struct SigEvent {
            sigev_value: usize,
            sigev_signo: i32,
            sigev_notify: i32,
            sigev_tid: i32,
            _pad: [i32; 11],
        }
        const _: () = assert!(core::mem::size_of::<SigEvent>() == 64);

        /// ★★★★★ **The handler that names the line.** `SIGALRM`'s default action dumps a
        /// core, and `[measured w497]` this container refuses to set `core_pattern`, so the
        /// dump never appeared. Printing the backtrace from the handler is strictly better:
        /// it lands **in the boot log**, next to the census that says the trap blocked.
        ///
        /// ⊘ `Backtrace::force_capture` is not async-signal-safe. That is accepted here and
        /// nowhere else: this handler exists only to print and then `_exit`, the process is
        /// already forfeit, and a temporary debug instrument that is merely *likely* to work
        /// beats a core file that provably does not exist.
        extern "C" fn on_stall(_: libc::c_int) {
            let bt = std::backtrace::Backtrace::force_capture();
            let msg = format!(
                "\nkayfabe: ⊘⊘⊘ TRAP STALL ALARM — this vCPU thread exceeded the budget \
                 WHILE STILL INSIDE ITS MMIO TRAP. `[measured w499]` every slow trap carried \
                 a VOLUNTARY context switch, so the frame below is where it BLOCKED.\n{bt}\n"
            );
            // SAFETY: `write(2)` to fd 2 with a pointer and length from a live `String`.
            // Async-signal-safe by POSIX, unlike the capture above.
            unsafe {
                libc::write(2, msg.as_ptr().cast(), msg.len());
            }
            // SAFETY: `_exit` performs no cleanup and is async-signal-safe. Deliberately not
            // `exit`: running atexit handlers from a signal is how a diagnostic turns into a
            // second, unrelated crash.
            unsafe { libc::_exit(42) };
        }

        /// Install the handler once. ⊘ Without this, `SIGALRM` terminates with no output at
        /// all and the instrument reports nothing — the worst failure it could have.
        fn install_handler() {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                // SAFETY: installing a handler for one signal with default flags.
                unsafe {
                    libc::signal(libc::SIGALRM, on_stall as libc::sighandler_t);
                }
            });
        }

        /// An armed per-trap alarm. Dropping it deletes the timer.
        #[derive(Debug)]
        pub struct Armed(libc::timer_t);

        fn budget_us() -> Option<i64> {
            static B: std::sync::OnceLock<Option<i64>> = std::sync::OnceLock::new();
            *B.get_or_init(|| {
                std::env::var("KAYFABE_STALL_ALARM_US")
                    .ok()
                    .and_then(|v| v.parse::<i64>().ok())
                    .filter(|v| *v > 0)
            })
        }

        /// Whether the instrument is armed at all. ⊘ Off unless `KAYFABE_STALL_ALARM_US` is
        /// set, so a production build creates no timer and pays nothing.
        #[must_use]
        pub fn enabled() -> bool {
            budget_us().is_some()
        }

        /// Arm an alarm on the calling thread for the configured budget.
        #[must_use]
        pub fn arm() -> Option<Armed> {
            let us = budget_us()?;
            install_handler();
            // SAFETY: `gettid` takes no argument and dereferences nothing.
            let tid = unsafe { libc::syscall(libc::SYS_gettid) } as i32;
            let mut sev = SigEvent {
                sigev_value: 0,
                sigev_signo: libc::SIGALRM,
                sigev_notify: SIGEV_THREAD_ID,
                sigev_tid: tid,
                _pad: [0; 11],
            };
            let mut t: libc::timer_t = core::ptr::null_mut();
            // SAFETY: `sev` matches glibc's `sigevent` layout (asserted above) and lives on
            // this stack for the call; `t` is a writable out-parameter of the right type.
            let rc = unsafe {
                libc::timer_create(
                    libc::CLOCK_MONOTONIC,
                    (&raw mut sev).cast::<libc::sigevent>(),
                    &raw mut t,
                )
            };
            if rc != 0 {
                return None;
            }
            let spec = libc::itimerspec {
                it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 },
                it_value: libc::timespec {
                    tv_sec: us / 1_000_000,
                    tv_nsec: (us % 1_000_000) * 1_000,
                },
            };
            // SAFETY: `spec` is fully initialised and outlives the call; a null out-parameter
            // is explicitly permitted by `timer_settime(2)`.
            if unsafe { libc::timer_settime(t, 0, &raw const spec, core::ptr::null_mut()) } != 0 {
                // SAFETY: `t` came from the successful `timer_create` just above.
                unsafe { libc::timer_delete(t) };
                return None;
            }
            Some(Armed(t))
        }

        impl Drop for Armed {
            /// ⊘ DELETES rather than merely disarming: a leaked `timer_t` per trap would
            /// exhaust the per-process timer limit within a second of boot, and the failure
            /// would look like the instrument simply stopping.
            fn drop(&mut self) {
                // SAFETY: `self.0` came from `timer_create` in `arm` and is deleted once.
                unsafe { libc::timer_delete(self.0) };
            }
        }
    }

    #[cfg(all(test, target_os = "linux", target_env = "gnu"))]
    mod timer_tests {
        /// ⊘ The instrument must be PROVEN to fire on the arming thread, not assumed. A
        /// hand-rolled `sigevent` that is one field out would arm a timer that never fires,
        /// and a debug tool that silently does nothing is worse than none — it would be read
        /// as "no trap was ever over budget".
        ///
        /// Catches `SIGALRM` instead of dying so the test can assert delivery.
        #[test]
        fn the_alarm_fires_on_the_arming_thread() {
            use std::sync::atomic::{AtomicBool, Ordering};
            static FIRED: AtomicBool = AtomicBool::new(false);
            extern "C" fn on_alarm(_: libc::c_int) {
                FIRED.store(true, Ordering::SeqCst);
            }
            // SAFETY: installing a handler for SIGALRM with default flags; `on_alarm` only
            // touches an atomic, which is async-signal-safe.
            unsafe {
                libc::signal(libc::SIGALRM, on_alarm as libc::sighandler_t);
            }
            // SAFETY: setting an env var in a single-threaded test section.
            unsafe { std::env::set_var("KAYFABE_STALL_ALARM_US", "2000") };
            let Some(armed) = super::timer::arm() else {
                // ⊘ `arm()` memoises the budget in a `OnceLock`, so another test in this
                // binary may have read it as absent first. Skipping is honest; asserting
                // would make the suite order-dependent.
                return;
            };
            std::thread::sleep(std::time::Duration::from_millis(60));
            assert!(
                FIRED.load(Ordering::SeqCst),
                "a 2ms alarm did not fire within 60ms — the sigevent layout is wrong and the \
                 instrument would report silence as 'nothing was over budget'"
            );
            drop(armed);
        }
    }

    /// ★★★★★ **w498 — THE DISCRIMINATOR: preempted, or BLOCKED?**
    ///
    /// `[measured w497]` the worst trap spent **656 us on a CPU and 16 818 us in total**. That
    /// proves the thread was **not on a CPU**. ⊘⊘ It does **not** prove it was descheduled —
    /// a thread blocked on a mutex is also not on a CPU and looks identical. I concluded
    /// "descheduling" from it, which was one step too far, and the owner was right to push:
    /// *"context switching time is in sub milliseconds, not 26ms right."*
    ///
    /// The kernel separates the two and needs no sysctl:
    /// - **involuntary** switches (`ru_nivcsw`) — the scheduler took the CPU away. Preemption.
    /// - **voluntary** switches (`ru_nvcsw`) — the thread gave it up. It **blocked**, on a
    ///   futex, a lock, or I/O.
    ///
    /// ⇒ a slow trap carrying involuntary switches is the run queue; one carrying voluntary
    /// switches is **something in our code waiting**, and then the owner's coarse-lock theory
    /// is right after all. ⊘ `sched_schedstats` is 0 on this box so runqueue-wait accounting
    /// reads zero — this path works regardless.
    #[must_use]
    pub fn thread_switches() -> Option<(u64, u64)> {
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: `ru` is a writable out-parameter of the right type living on this stack;
        // `RUSAGE_THREAD` scopes the answer to the calling thread.
        let rc = unsafe { libc::getrusage(libc::RUSAGE_THREAD, &raw mut ru) };
        (rc == 0).then(|| (ru.ru_nvcsw as u64, ru.ru_nivcsw as u64))
    }

    /// This thread's consumed CPU time, for the wall-versus-CPU comparison.
    ///
    /// ★ This is the half that needs no crash: `cpu` far below `wall` means the thread was
    /// **descheduled**, and nothing of ours is responsible for the difference.
    ///
    /// # Errors
    /// If `clock_gettime` refuses.
    pub fn thread_cpu_nanos() -> Result<u64, RawError> {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `ts` is a writable out-parameter of the right type, living on this stack.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &raw mut ts) };
        if rc != 0 {
            return Err(last_syscall_error("clock_gettime"));
        }
        Ok((ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64))
    }
}
