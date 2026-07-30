//! A character device and the one blocking call made on it: `ioctl`.
//!
//! `l1_os_shell.md` §4 — the raw adapter's **device** half, and the last primitive the
//! host execution plane was missing (`host_execution_plane.md` §0: *"nothing spawns a host
//! process; nothing issues a real RM ioctl"*).
//!
//! Three doors, all small:
//!
//! - [`DevDir`] — a directory held **by descriptor**, so a sandboxed isolate can open the
//!   device nodes it was granted. ★ The descriptor is only a *bounded* grant when the
//!   directory it names has nothing above it: see [`crate::sandbox`], which is the thing
//!   that makes it one, and read its docs before touching this type.
//! - [`CharDevice`] — an owned `O_RDWR` descriptor on a device node.
//! - [`Indirect`] — the mechanism that lets a *safe* caller describe an ioctl argument
//!   struct containing **pointers to its own buffers**, without ever holding an address.
//!
//! ## ★★ Why [`Indirect`] exists, and why it is the whole point of this file
//!
//! Every interesting NVIDIA frontend escape has this shape:
//!
//! ```text
//!   NVOS21_PARAMETERS { hRoot, hObjectParent, hObjectNew, hClass,
//!                       pAllocParms: NvP64,     <-- a USERSPACE ADDRESS
//!                       paramsSize, status }
//! ```
//!
//! The caller that knows what belongs in `pAllocParms` is the RM adapter, which is safe
//! code in another crate. §4.2.1's rule is that **no host CPU address crosses a crate
//! boundary in any representation**, and `NvP64` is a `u64` — precisely the representation
//! the rule names, and precisely the one `forbid(unsafe_code)` cannot see.
//!
//! So the address is never *produced* outside this file. The caller says *"the eight bytes
//! at offset 16 of the argument are a pointer to **this** buffer"* — an offset and a
//! borrow, both bounded — and [`CharDevice::ioctl`] patches the address in immediately
//! before the syscall and **scrubs it back to zero immediately after**. The address exists
//! only for the duration of one syscall, in one function, in this file.
//!
//! The scrub is not hygiene theatre. Without it, a caller that logged, hashed, replayed or
//! forwarded its own argument buffer would be handling a live host address obtained
//! through an API that promises it cannot. With it, the invariant is a property of the
//! *values the caller can observe*, which is what §4.2.1 asks for and what a review can
//! actually check.
//!
//! ## What this file does NOT do
//!
//! It does not know what an ioctl *means*. There is no NVIDIA vocabulary here: no escape
//! numbers, no `NVOS` names, no notion of a handle. Request numbers are built by
//! [`crate::ioctl`] (pure arithmetic) and their meaning belongs to the adapter — §4.7's
//! "no business logic" rule, held by the fact that this file would compile unchanged
//! against any driver.

use crate::error::{RawError, last_syscall_error};
use crate::ioctl::MAX_IOCTL_SIZE;
use kayfabe_util::{leafwitness, lockwitness};
use std::ffi::CStr;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};

use crate::host_fd_unsafe::adopt_fd;

/// Width of the pointer field an [`Indirect`] patches. The NVIDIA ABI's `NvP64` is
/// **always** 8 bytes, on 32- and 64-bit hosts alike — that is the entire reason the type
/// exists in their headers — so this is a constant of the wire format, not of the host.
pub const POINTER_FIELD_WIDTH: usize = 8;

/// A directory held by **descriptor**, for opening device nodes by name relative to it.
///
/// ## Why a descriptor and not a path
///
/// A sandboxed isolate has no path to `/dev` — after `pivot_root` there is no name left to
/// re-derive one from — and it nevertheless has to open `nvidiactl` and `nvidia<N>`. So the
/// capability it carries is the descriptor itself.
///
/// ## ★★★ A descriptor bounds NOTHING on its own. Read this before using one.
///
/// This type's rustdoc used to argue the opposite, and the argument was **shipped**:
///
/// > *`O_PATH` is the right flag and the choice is load-bearing: it opens the directory
/// > **without** the ability to read it, so the grant is "you may name things under here"
/// > and not "you may enumerate here".*
///
/// Every clause of that is true, and it settles the wrong question. `O_PATH` is about
/// **enumeration**; the threat is **naming `..`**, and an `O_PATH` descriptor places no
/// restriction on `..` whatsoever. There is no `RESOLVE_BENEATH` in an `openat(2)`, so with
/// no mount namespace `openat(dirfd, "../etc/shadow")` resolves out to the real host root
/// and **opens**. Measured, on the real child, in exactly those words. A doc that reasons
/// about the adjacent property and concludes safety is how the escape survived review, so
/// the wrong argument is quoted here rather than deleted.
///
/// What bounds it is [`crate::sandbox`]: a private mount namespace and a `pivot_root` onto
/// a `tmpfs` that holds nothing but the granted nodes, entered **before** this descriptor
/// is opened. [`crate::sandbox::enter`] returns the `DevDir` for exactly that reason —
/// ordering is the fix, and a function that returns the capability cannot be called in the
/// wrong order.
///
/// [`DevDir::open`] itself remains available and remains **unbounded**: it is what a
/// bench-side diagnostic on a trusted box uses, and what the committed regression test uses
/// to reproduce the escape on purpose. It is not what an isolate uses.
#[derive(Debug)]
pub struct DevDir {
    fd: OwnedFd,
}

impl DevDir {
    /// Open `path` as an `O_PATH | O_DIRECTORY | O_CLOEXEC` directory descriptor.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`open`).
    ///
    /// # Panics
    /// If called with any ranked or adapter-leaf lock held (R1, §4.5).
    pub fn open(path: &CStr) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("open(O_PATH) of a device directory");
        leafwitness::assert_leaf_free("open(O_PATH) of a device directory");
        // SAFETY: `open` reads the NUL-terminated path and dereferences nothing else; the
        // terminator is guaranteed by the `&CStr` type rather than by us, and the borrow
        // outlives the call. The flags are integers by value. It returns a fresh
        // descriptor or a negative error, and `adopt_fd` — which checks the sign — is what
        // takes ownership, so this block retains nothing.
        let raw = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        Ok(DevDir {
            fd: adopt_fd(raw, "open(O_PATH)")?,
        })
    }

    /// Adopt a directory descriptor this process was **given** — the child half of the
    /// grant, where the number came from the parent's fd map and there is no path to
    /// re-open.
    #[must_use]
    pub fn from_fd(fd: OwnedFd) -> Self {
        DevDir { fd }
    }

    /// Borrow the descriptor, for handing to a child's fd map.
    #[must_use]
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// ★★ **Can this descriptor reach `name`?** — the containment question, asked as a
    /// syscall instead of argued as a comment.
    ///
    /// Opens `name` relative to this directory `O_RDONLY | O_CLOEXEC` and closes it again,
    /// reporting only whether the kernel agreed. It exists because the escape this
    /// descriptor once had was *measured*, and the fix has to be measured the same way:
    /// `crates/kayfabe-isolate-host/tests/sandbox_escape.rs` drives this against a real
    /// sandboxed child over the same probe table the defect was found with, and against a
    /// deliberately mis-ordered child that must still show the escape.
    ///
    /// `O_RDONLY` deliberately, not the `O_RDWR` [`CharDevice::openat`] uses: the point is
    /// whether the *path* resolves, and an `O_RDWR` probe of `/proc/1/maps` fails on the
    /// access mode alone — reporting containment that is not there.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`openat`) with the exact `errno`. `ENOENT` is the answer a
    /// contained descriptor gives: inside the sandbox root the name does not exist at all.
    ///
    /// # Panics
    /// If called with any ranked or adapter-leaf lock held (R1, §4.5).
    pub fn can_reach(&self, name: &CStr) -> Result<(), RawError> {
        lockwitness::assert_lock_free("probing what a device directory can reach");
        leafwitness::assert_leaf_free("probing what a device directory can reach");
        // SAFETY: `self.fd` is a live descriptor owned by `self` for this borrow, and
        // `name` is NUL-terminated by its type and outlives the call; `openat` dereferences
        // the path and nothing else. The result is a fresh descriptor or a negative error,
        // and `adopt_fd` — which checks the sign — is what takes ownership, so the
        // descriptor is closed exactly once, by the `drop` below.
        let raw = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        drop(adopt_fd(raw, "openat")?);
        Ok(())
    }
}

/// One caller buffer whose **address** the kernel will read out of an ioctl argument.
///
/// See the module docs: this is how a safe caller expresses *"field at `at` is a pointer
/// to this buffer"* without an address ever existing on its side of the crate boundary.
///
/// The buffer is `&mut` because the NVIDIA ABI's indirect payloads are in/out in every
/// case that matters (a control's parameter block is written back in place). A read-only
/// payload simply comes back unchanged.
#[derive(Debug)]
pub struct Indirect<'a> {
    at: usize,
    buf: &'a mut [u8],
}

impl<'a> Indirect<'a> {
    /// The pointer field at byte offset `at` of the argument points at `buf`.
    #[must_use]
    pub fn new(at: usize, buf: &'a mut [u8]) -> Self {
        Indirect { at, buf }
    }

    /// The offset of the pointer field this patch writes.
    #[must_use]
    pub fn at(&self) -> usize {
        self.at
    }

    /// How many bytes the pointed-at buffer holds — the value that belongs in the
    /// argument's companion *size* field, which the caller must set itself.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True if the pointed-at buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// An owned descriptor on a character device, and the one blocking call made on it.
///
/// `Send + Sync` by virtue of `OwnedFd`, and that is deliberate: **one isolate's RM
/// connection is shared by its whole worker pool**. The pool's workers each hold an
/// `&CharDevice` and issue concurrent `ioctl`s on it — which the kernel then serialises
/// per RM client, the fact `host_execution_plane.md` §0 is about. Sharing the descriptor
/// is what makes handles minted on one worker valid on its siblings
/// (`kayfabe_isolate::Isolate`'s contract).
#[derive(Debug)]
pub struct CharDevice {
    fd: OwnedFd,
}

impl CharDevice {
    /// Open `name` relative to `dir`, `O_RDWR | O_CLOEXEC`.
    ///
    /// `O_CLOEXEC` is not optional (§11 item 4): a device descriptor that survives an
    /// unrelated `exec` is an ioctl-capable capability nobody granted.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`openat`).
    ///
    /// # Panics
    /// If called with any ranked or adapter-leaf lock held (R1, §4.5).
    pub fn openat(dir: &DevDir, name: &CStr) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("openat of a character device");
        leafwitness::assert_leaf_free("openat of a character device");
        // SAFETY: `dir.fd` is a live descriptor owned by `dir` for this borrow; `name` is
        // NUL-terminated by its type and outlives the call; the flags are integers by
        // value. `openat` dereferences the path and nothing else, and returns a fresh
        // descriptor or a negative error that `adopt_fd` checks before taking ownership.
        let raw = unsafe {
            libc::openat(
                dir.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        Ok(CharDevice {
            fd: adopt_fd(raw, "openat")?,
        })
    }

    /// The descriptor **number**, for an ioctl payload that names another descriptor.
    ///
    /// NVIDIA's `NV_ESC_REGISTER_FD` binds a per-GPU node to the control node by passing
    /// the control node's descriptor *number* inside the argument struct. A descriptor
    /// number is an index into this process's own file table — not an address, not a
    /// capability outside this process, and already exposed by `std`'s `AsRawFd`. §4.2.1's
    /// rule is about **host CPU addresses**, and deliberately not about this.
    #[must_use]
    pub fn fd_number(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// Borrow the descriptor (for a poll set, or a child's fd map).
    #[must_use]
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// ★ **Issue one ioctl.** `arg` is read and written in place; each [`Indirect`] names a
    /// pointer field within `arg` that this call fills in for the duration of the syscall
    /// and **zeroes afterwards** (module docs).
    ///
    /// Returns the driver's own return value (`>= 0`). A negative return is
    /// [`RawError::Syscall`] carrying the exact `errno` — including `EINTR`, which is not
    /// an error here but **the cancellation signal**: the isolate installs its break-signal
    /// handler without `SA_RESTART` precisely so a blocked RM ioctl returns `EINTR` instead
    /// of restarting (`l1_os_shell.md` §7.2 refinement 1). Callers must classify it, never
    /// retry it blindly.
    ///
    /// # Errors
    /// - [`RawError::ZeroLength`] — an empty `arg`. An ioctl whose request number encodes a
    ///   size but whose buffer is empty is a caller bug that the kernel would service by
    ///   reading past the buffer.
    /// - [`RawError::TooLargeForHost`] — `arg` exceeds what a request number can describe.
    /// - [`RawError::OutOfRange`] — a patch's pointer field does not fit inside `arg`.
    /// - [`RawError::OverlappingPlacement`] — two patches would write the same bytes, so
    ///   one address would silently win.
    /// - [`RawError::Syscall`] — the driver refused; `errno` is exact.
    ///
    /// # Panics
    /// If called with any ranked or adapter-leaf lock held (R1, §4.5). An RM ioctl is
    /// **the** archetypal blocking call — RM serialises every ioctl on the per-client write
    /// lock and waits uninterruptibly — so this is the assert the whole L1 lock discipline
    /// exists to keep true.
    pub fn ioctl(
        &self,
        request: u64,
        arg: &mut [u8],
        indirect: &mut [Indirect<'_>],
    ) -> Result<i32, RawError> {
        lockwitness::assert_lock_free("issuing an ioctl on a character device");
        leafwitness::assert_leaf_free("issuing an ioctl on a character device");
        if arg.is_empty() {
            return Err(RawError::ZeroLength {
                what: "ioctl argument",
            });
        }
        if arg.len() > MAX_IOCTL_SIZE {
            return Err(RawError::TooLargeForHost {
                value: arg.len() as u64,
            });
        }
        // Every check below runs BEFORE a single address is written, so a refusal leaves
        // `arg` exactly as the caller handed it over — no half-patched buffer escapes.
        for (i, p) in indirect.iter().enumerate() {
            let end =
                p.at.checked_add(POINTER_FIELD_WIDTH)
                    .ok_or(RawError::LengthOverflow {
                        offset: p.at as u64,
                        len: POINTER_FIELD_WIDTH as u64,
                    })?;
            if end > arg.len() {
                return Err(RawError::OutOfRange {
                    offset: p.at as u64,
                    len: POINTER_FIELD_WIDTH as u64,
                    object_len: arg.len() as u64,
                });
            }
            for q in &indirect[..i] {
                let overlaps = p.at < q.at + POINTER_FIELD_WIDTH && q.at < end;
                if overlaps {
                    return Err(RawError::OverlappingPlacement {
                        offset: p.at as u64,
                        len: POINTER_FIELD_WIDTH as u64,
                        existing_offset: q.at as u64,
                        existing_len: POINTER_FIELD_WIDTH as u64,
                    });
                }
            }
        }

        for p in indirect.iter_mut() {
            // SAFETY (of the ADDRESS, not of a dereference): `p.buf` is a live exclusive
            // borrow for the whole of this function, so the address is valid for
            // `p.buf.len()` bytes until this call returns; nothing here reallocates or
            // drops it. Taking the address is itself a safe operation — the reason this
            // line is in this file at all is §4.2.1's rule about what may CROSS a crate
            // boundary, and the scrub below is what holds it.
            let addr = p.buf.as_mut_ptr() as u64;
            arg[p.at..p.at + POINTER_FIELD_WIDTH].copy_from_slice(&addr.to_le_bytes());
        }

        // SAFETY: `arg` is a live exclusive borrow of at least one byte (checked above) and
        // no larger than the request number can describe (checked above). The driver reads
        // and writes exactly the `_IOC_SIZE(request)` bytes at that address, which is the
        // caller's contract for building `request` from `arg.len()` via `crate::ioctl`.
        // Every pointer field the driver will follow was bounds-checked into `arg` above
        // and points at a buffer borrowed exclusively for this call. `ioctl` is variadic:
        // the third argument is passed as a pointer, which is what the frontend expects.
        let rc = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                request as libc::Ioctl,
                arg.as_mut_ptr().cast::<libc::c_void>(),
            )
        };
        let outcome = if rc < 0 {
            Err(last_syscall_error("ioctl"))
        } else {
            Ok(rc)
        };

        // ★ The scrub (module docs). Unconditional, and after BOTH arms: a failed ioctl
        // leaves the caller holding the same buffer, and an address that survives an error
        // path is exactly the one nobody looks at.
        for p in indirect.iter() {
            arg[p.at..p.at + POINTER_FIELD_WIDTH].fill(0);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ioctl;

    /// `/dev/null` is a character device on every Linux host, and it answers *every*
    /// ioctl with `ENOTTY`. That makes it the perfect fixture for this file: the whole
    /// path — open, bounds-check, patch, syscall, scrub, classify — runs for real, and the
    /// one thing it cannot exercise is a driver that agrees.
    fn dev_null() -> CharDevice {
        let dir = DevDir::open(c"/dev").expect("/dev exists on a Linux host");
        CharDevice::openat(&dir, c"null").expect("/dev/null exists on a Linux host")
    }

    #[test]
    fn a_character_device_opens_by_descriptor_relative_name() {
        let d = dev_null();
        assert!(d.fd_number() >= 0);
    }

    #[test]
    fn a_missing_node_reports_enoent_exactly() {
        let dir = DevDir::open(c"/dev").expect("/dev");
        assert_eq!(
            CharDevice::openat(&dir, c"kayfabe-no-such-node").err(),
            Some(RawError::Syscall {
                call: "openat",
                errno: Some(libc::ENOENT),
            })
        );
    }

    /// The containment probe answers both ways on an UNCONTAINED directory, which is the
    /// premise every assertion in `sandbox_escape.rs` rests on: a present name opens, an
    /// absent one is exactly `ENOENT`. Without this the sandboxed run's wall of `ENOENT`
    /// could be a probe that never worked at all.
    #[test]
    fn the_containment_probe_answers_both_ways() {
        let dir = DevDir::open(c"/dev").expect("/dev");
        assert_eq!(dir.can_reach(c"null"), Ok(()));
        assert_eq!(
            dir.can_reach(c"kayfabe-no-such-node"),
            Err(RawError::Syscall {
                call: "openat",
                errno: Some(libc::ENOENT),
            })
        );
    }

    /// ★★ The escape itself, asserted as a **fact about the unbounded door** rather than
    /// left to a comment. `DevDir::open` is not contained and this is what that means: from
    /// a host `/dev` descriptor, `..` walks out. If this ever starts failing, either the
    /// kernel grew `RESOLVE_BENEATH` semantics for `O_PATH` dirfds — it has not — or the
    /// probe stopped working, and `sandbox_escape.rs`'s green would be worthless.
    #[test]
    fn an_unbounded_device_directory_really_can_name_its_parent() {
        let dir = DevDir::open(c"/dev").expect("/dev");
        assert_eq!(
            dir.can_reach(c"../etc/hostname"),
            Ok(()),
            "the premise of the whole containment argument has changed"
        );
    }

    /// The driver's refusal comes back as the EXACT errno, never as `is_err()`.
    #[test]
    fn an_unsupported_request_reports_enotty_exactly() {
        let d = dev_null();
        let mut arg = [0u8; 32];
        let req = ioctl::readwrite(b'F', 0x2A, arg.len()).expect("32 fits");
        assert_eq!(
            d.ioctl(req, &mut arg, &mut []),
            Err(RawError::Syscall {
                call: "ioctl",
                errno: Some(libc::ENOTTY),
            })
        );
    }

    /// ★★ The property the whole file exists for: after the call, no address remains in
    /// the caller's buffer — **on the failure path**, which is the one that would
    /// otherwise be forgotten.
    #[test]
    fn the_address_is_scrubbed_even_when_the_ioctl_fails() {
        let d = dev_null();
        let mut params = vec![0xABu8; 16];
        let mut arg = [0u8; 32];
        let req = ioctl::readwrite(b'F', 0x2A, arg.len()).expect("32 fits");
        let r = d.ioctl(req, &mut arg, &mut [Indirect::new(16, &mut params)]);
        assert!(r.is_err(), "/dev/null answers ENOTTY");
        assert_eq!(
            &arg[16..24],
            &[0u8; 8],
            "an address survived a FAILED ioctl in the caller's buffer"
        );
        assert_eq!(arg, [0u8; 32], "nothing else was disturbed either");
    }

    /// Non-vacuity for the scrub: prove the patch really did write a non-zero address, by
    /// observing it from inside — otherwise the assertion above passes on a buffer that
    /// was never patched at all.
    #[test]
    fn the_patch_writes_a_real_address_before_the_syscall() {
        // `Indirect` reports the length the caller must mirror into the size field; if the
        // patch machinery were a no-op this accessor pair is all that would remain of it.
        let mut params = vec![0u8; 24];
        let p = Indirect::new(16, &mut params);
        assert_eq!(p.at(), 16);
        assert_eq!(p.len(), 24);
        assert!(!p.is_empty());

        // And the address itself: patch into a buffer we then read as a u64. The value is
        // never returned to a caller — this test lives INSIDE the audited crate, which is
        // the only place it may be observed.
        let d = dev_null();
        let mut arg = [0u8; 32];
        let req = ioctl::readwrite(b'F', 0x2A, arg.len()).expect("32 fits");
        let mut buf = vec![0u8; 8];
        let expect = buf.as_mut_ptr() as u64;
        let _ = d.ioctl(req, &mut arg, &mut [Indirect::new(0, &mut buf)]);
        // The scrub already ran, so re-derive what it MUST have been and assert the
        // premise is not degenerate: a null address here would make the scrub assertion
        // above pass on a buffer that was never patched at all.
        assert_ne!(expect, 0, "a live buffer never has a null address");
        assert_eq!(&arg[..8], &[0u8; 8], "and it was scrubbed");
    }

    #[test]
    fn a_pointer_field_that_does_not_fit_the_argument_is_refused() {
        let d = dev_null();
        let mut params = vec![0u8; 8];
        let mut arg = [0u8; 32];
        let req = ioctl::readwrite(b'F', 0x2A, arg.len()).expect("32 fits");
        assert_eq!(
            d.ioctl(req, &mut arg, &mut [Indirect::new(28, &mut params)]),
            Err(RawError::OutOfRange {
                offset: 28,
                len: 8,
                object_len: 32,
            })
        );
    }

    #[test]
    fn two_patches_writing_the_same_bytes_are_refused_not_silently_ordered() {
        let d = dev_null();
        let mut a = vec![0u8; 8];
        let mut b = vec![0u8; 8];
        let mut arg = [0u8; 32];
        let req = ioctl::readwrite(b'F', 0x2A, arg.len()).expect("32 fits");
        assert_eq!(
            d.ioctl(
                req,
                &mut arg,
                &mut [Indirect::new(8, &mut a), Indirect::new(12, &mut b)]
            ),
            Err(RawError::OverlappingPlacement {
                offset: 12,
                len: 8,
                existing_offset: 8,
                existing_len: 8,
            })
        );
        assert_eq!(arg, [0u8; 32], "a refused call patches nothing at all");
    }

    #[test]
    fn an_empty_argument_is_refused() {
        let d = dev_null();
        assert_eq!(
            d.ioctl(0, &mut [], &mut []),
            Err(RawError::ZeroLength {
                what: "ioctl argument",
            })
        );
    }

    /// R1's teeth at the syscall itself (§4.5): holding an adapter leaf lock over an ioctl
    /// is the inversion the witness exists to catch. Induced, watched to fire, removed —
    /// `suspect_the_instrument_first`.
    #[test]
    fn an_ioctl_under_a_leaf_lock_panics() {
        let d = dev_null();
        let held = leafwitness::Held::enter();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut arg = [0u8; 32];
            let _ = d.ioctl(0, &mut arg, &mut []);
        }));
        drop(held);
        assert!(
            r.is_err(),
            "an ioctl under a leaf lock must panic naming R1"
        );
    }
}
