//! Two syscall wrappers: `capget` and `capset`, each on the **calling thread** only. Nothing
//! else belongs in this file.
//!
//! `l1_os_shell.md` §4.1.1 asks these files to be *small and boring*. The policy — which bit is
//! cleared, around which call, and what a caller does when the kernel refuses — lives in
//! [`crate::capability`]. This file reads and writes the three 64-bit sets and nothing more.
//!
//! # The kernel ABI, and why there is no `repr(C)` type here
//!
//! `include/uapi/linux/capability.h`: `_LINUX_CAPABILITY_VERSION_3 = 0x20080522`, a header
//! `{ __u32 version; int pid; }`, and for version 3 **two** data records
//! `{ __u32 effective, permitted, inheritable; }` — capabilities 0..31 in the first, 32..63 in
//! the second. They are written here as `[u32; 2]` and `[u32; 6]`: the same size, alignment and
//! field order, with no layout type to keep in step with the header. `pid = 0` names the
//! calling thread for both calls (`capget(2)`; `capset(2)` may only change the caller's own sets).

use crate::capability::ThreadCaps;

/// `_LINUX_CAPABILITY_VERSION_3` — the 64-bit, two-record form.
const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;

fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// The calling thread's effective, permitted and inheritable sets, or the `errno` of the refusal.
pub(crate) fn capget_self() -> Result<ThreadCaps, i32> {
    let mut hdr: [u32; 2] = [LINUX_CAPABILITY_VERSION_3, 0];
    let mut data: [u32; 6] = [0; 6];
    // SAFETY: `capget(hdr, data)` with a version-3 header reads the 8-byte header and writes
    // exactly two 12-byte records — 24 bytes — into `data`, which is a 24-byte array owned by
    // this frame and alive across the call. On a version mismatch it writes the preferred version
    // into `hdr[0]`, which is also owned, writable and 8 bytes long. `pid = 0` names the calling
    // thread. The kernel keeps neither pointer after the call returns; failure is reported by
    // the return value, which is checked.
    let rc = unsafe { libc::syscall(libc::SYS_capget, hdr.as_mut_ptr(), data.as_mut_ptr()) };
    if rc != 0 {
        return Err(last_errno());
    }
    Ok(ThreadCaps::from_records(data))
}

/// Replace the calling thread's three sets with `caps`, or return the `errno` of the refusal.
pub(crate) fn capset_self(caps: ThreadCaps) -> Result<(), i32> {
    let mut hdr: [u32; 2] = [LINUX_CAPABILITY_VERSION_3, 0];
    let data: [u32; 6] = caps.to_records();
    // SAFETY: `capset(hdr, data)` with a version-3 header reads the 8-byte header and exactly two
    // 12-byte records from `data` (24 bytes, owned by this frame, alive across the call), and
    // may write only `hdr[0]` (owned and writable) on a version mismatch. `pid = 0` restricts
    // the change to the calling thread's own credentials; the kernel enforces that the new sets
    // are a legal transition (effective within permitted, permitted not raised) and refuses
    // otherwise through the return value, which is checked. No pointer is kept after the call.
    let rc = unsafe { libc::syscall(libc::SYS_capset, hdr.as_mut_ptr(), data.as_ptr()) };
    if rc != 0 {
        return Err(last_errno());
    }
    Ok(())
}
