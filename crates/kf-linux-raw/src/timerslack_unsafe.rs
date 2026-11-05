//! Two syscall wrappers: `prctl(PR_SET_TIMERSLACK)` and `prctl(PR_GET_TIMERSLACK)`, for the
//! calling thread. Nothing else belongs in this file.
//!
//! ★ 2026-11-03 (walk wake, `traces/walk_wake_20261103/`): a thread that sleeps for tens of
//! microseconds — the VA worker looking at a pending GPU walk — is otherwise woken up to the
//! kernel's default **50 us** late (`timer_slack_ns`, applied to every `epoll_wait`-family
//! timeout of a normal-priority thread). That slack is as large as the wait itself. It is a
//! per-thread value and changes nothing but how late this thread's own timeouts may fire.

/// Set the calling thread's timer slack to `ns` nanoseconds. Returns whether the kernel
/// accepted it.
///
/// ⊘ `0` is refused **here**: to the kernel it means "reset to the thread's default", which is
/// the opposite of what a caller asking for zero slack wants.
pub fn set_current_thread_timer_slack_ns(ns: u64) -> bool {
    if ns == 0 {
        return false;
    }
    // `unsigned long` is 64 bits on every target this crate builds for (x86_64, aarch64).
    let arg: libc::c_ulong = ns;
    // SAFETY: `prctl(PR_SET_TIMERSLACK, ns)` takes the slack by value, names only the CALLING
    // thread, and dereferences no caller memory; the unused trailing arguments are zero as the
    // man page requires. Its only failures are reported by the return value, which is checked.
    let rc = unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, arg, 0 as libc::c_ulong, 0, 0) };
    rc == 0
}

/// The calling thread's current timer slack in nanoseconds, or `None` if the kernel refused
/// to say.
#[must_use]
pub fn current_thread_timer_slack_ns() -> Option<u64> {
    // SAFETY: `prctl(PR_GET_TIMERSLACK)` returns the slack by value and writes through no
    // pointer; a negative return is the documented failure and is checked.
    let rc = unsafe { libc::prctl(libc::PR_GET_TIMERSLACK, 0 as libc::c_ulong, 0, 0, 0) };
    u64::try_from(rc).ok()
}
