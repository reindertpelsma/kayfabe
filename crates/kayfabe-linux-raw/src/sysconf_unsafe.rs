//! One syscall wrapper: `sysconf(_SC_PAGESIZE)`. Nothing else belongs in this file.
//!
//! `l1_os_shell.md` §4.1.1 asks these files to be *small and boring*, on the grounds that
//! a safety argument needing a paragraph of GPU or guest semantics is a sign a decision
//! leaked down a layer. This one is one line and one invariant.
//!
//! The **value** is validated by [`crate::page_size`], not here: this file's job is to
//! obtain the number, and a file that both calls the OS and decides what the answer means
//! is two audits pretending to be one.

/// The host page size the OS reports, unvalidated and possibly absurd.
///
/// Deliberately returns the raw `i64` rather than a validated newtype — the caller owns
/// the "is this a plausible page size" decision (§5.2 item 1), and preserving the number
/// the OS actually gave is what makes its refusal message evidence rather than a guess.
pub(crate) fn sc_pagesize() -> i64 {
    // SAFETY: `sysconf` reads a process-global constant the kernel published at exec.
    // Its only argument is an integer *name* — it dereferences no caller memory, writes
    // through no pointer, and returns its answer by value, so there is no buffer, length
    // or lifetime for this call to get wrong. `_SC_PAGESIZE` is defined on every Linux
    // target this crate compiles for (the crate root rejects non-Linux), so the
    // "unrecognised name" branch that is the only documented failure mode is
    // unreachable; a nonsensical value is still possible and is the caller's business,
    // never this file's.
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) }
}
