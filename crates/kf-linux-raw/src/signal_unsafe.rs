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
