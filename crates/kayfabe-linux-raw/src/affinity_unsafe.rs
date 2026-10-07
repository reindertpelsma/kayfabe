//! Two syscall wrappers: `sched_setaffinity` and `sched_getaffinity`, for the calling
//! thread. Nothing else belongs in this file.
//!
//! `l1_os_shell.md` §4.1.1 asks these files to be *small and boring*. This one is two calls
//! and one invariant: the mask is built here, from a caller-supplied set of core indices,
//! and is never handed in as raw memory.
//!
//! # ★★★ WHY A DIAGNOSTIC NEEDS THIS AT ALL
//!
//! In kayfabe **each guest vCPU is a host thread**, so the concurrency the product has to
//! survive is *those* threads contending — a bounded set, pinned by the VMM, preempting each
//! other on a small number of cores. A stress test whose threads land wherever the scheduler
//! puts them samples a *different* topology: on a wide box they spread across idle cores, run
//! past each other, and never interleave **inside** a critical section. ⊘ That is the failure
//! mode a lock-order bug hides in, and an unpinned run can be green for the whole of it.
//!
//! ⚠ This is not a "make it faster" knob. Pinning here **removes** parallelism on purpose.

/// The cores the calling thread is currently allowed to run on, or `None` if the kernel
/// refused to say.
///
/// ⊘ Returns the set rather than a count: *"this thread may run on 19 cores"* and *"this
/// thread may run on cores 3, 7 and 11"* are different facts, and a caller that only gets the
/// cardinality cannot print an attributable pinning line.
pub fn current_cores() -> Option<Vec<usize>> {
    // SAFETY: `CPU_ZERO` writes only through the pointer we pass, which is to a `cpu_set_t`
    // this function owns on its own stack and keeps alive across both calls. Nothing else
    // aliases it.
    let mut set: libc::cpu_set_t = unsafe { core::mem::zeroed() };
    // SAFETY: `sched_getaffinity(0, size, &mut set)` names the CALLING thread (`pid == 0`),
    // is told the exact byte size of the object it may write, and writes nothing else. The
    // object outlives the call. Its only documented failures are reported by the return
    // value, which is checked.
    let rc = unsafe {
        libc::sched_getaffinity(0, core::mem::size_of::<libc::cpu_set_t>(), &raw mut set)
    };
    if rc != 0 {
        return None;
    }
    let mut out = Vec::new();
    for c in 0..(core::mem::size_of::<libc::cpu_set_t>() * 8) {
        // SAFETY: `CPU_ISSET` reads the bit at `c` of a `cpu_set_t` we own, and `c` is
        // bounded by the object's own bit width, computed from `size_of` on the same type.
        if unsafe { libc::CPU_ISSET(c, &set) } {
            out.push(c);
        }
    }
    Some(out)
}

/// Restrict the **calling thread** to `cores`. Returns whether the kernel accepted it.
///
/// ⊘ A `bool` and not a `Result`: the only thing a caller can do with a refusal is say so in
/// its log, and the one error this can produce that is not a programming mistake is a
/// sandbox forbidding the call. ⊘ An empty `cores` is refused **here** rather than passed on,
/// because an empty mask is `EINVAL` and the caller would read that as "pinning is not
/// available" instead of "I asked for nothing".
pub fn pin_current_thread(cores: &[usize]) -> bool {
    if cores.is_empty() {
        return false;
    }
    let bits = core::mem::size_of::<libc::cpu_set_t>() * 8;
    if cores.iter().any(|&c| c >= bits) {
        return false;
    }
    // SAFETY: as `current_cores` — an owned, zeroed `cpu_set_t` on this function's stack.
    let mut set: libc::cpu_set_t = unsafe { core::mem::zeroed() };
    for &c in cores {
        // SAFETY: `c` is bounded above by `bits`, checked immediately above, and `set` is
        // owned by this frame and unaliased.
        unsafe { libc::CPU_SET(c, &mut set) };
    }
    // SAFETY: `sched_setaffinity(0, size, &set)` names the CALLING thread (`pid == 0`), is
    // told the exact byte size of the object it may read, reads nothing else, and the object
    // outlives the call. It writes through no caller pointer at all.
    let rc = unsafe {
        libc::sched_setaffinity(0, core::mem::size_of::<libc::cpu_set_t>(), &raw const set)
    };
    rc == 0
}
