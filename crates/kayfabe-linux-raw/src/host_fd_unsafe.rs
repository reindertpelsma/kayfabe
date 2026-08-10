//! Owned host descriptors: adopting one, `memfd`, and the notify descriptor.
//!
//! `l1_os_shell.md` §4 — the raw adapter's descriptor half. Three doors, all small and
//! all boring, which is what §4.7 asks of this crate:
//!
//! - [`adopt_fd`] — the one place a raw integer descriptor becomes an `OwnedFd`. It is
//!   `pub(crate)` and every other `*_unsafe.rs` file in this crate calls it rather than
//!   repeating the relaxation, so the "a descriptor now has an owner" argument is made
//!   **once**.
//! - [`SharedRam`] — a sealed `memfd`. This is §4.4.1's *shareable backing*, made real:
//!   guest RAM that a second process can map is guest RAM that was **created** shareable,
//!   and nothing else can be retrofitted into one.
//! - [`Notifier`] — a counter-shaped edge. Its whole reason to exist here is
//!   [`Notifier::signal_under_lock`], the **named** exception §4.5 reserves for the
//!   deliberately non-blocking discharges: `grep -rn _under_lock` enumerates every
//!   in-lock syscall in the workspace, and the enumeration is the property we want — not
//!   *"there are none"* but *"there are exactly these, and each was argued"*.

use crate::error::{RawError, last_syscall_error};
use kayfabe_util::{leafwitness, lockwitness};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

/// Adopt a raw descriptor the kernel just returned, or report the `errno` that came
/// instead. **The only `from_raw_fd` in this crate.**
pub(crate) fn adopt_fd(raw: libc::c_int, call: &'static str) -> Result<OwnedFd, RawError> {
    if raw < 0 {
        return Err(last_syscall_error(call));
    }
    // SAFETY: `raw` is non-negative (checked on the line above) and was returned by the
    // syscall named in `call`, which returns either a NEW descriptor owned by this
    // process or a negative error. Every call site is `adopt_fd(libc::…(…), "…")` — the
    // value is consumed on the same expression that produced it — so this is the unique
    // owner and the `OwnedFd` will `close` it exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

// =====================================================================================
// SharedRam — the shareable backing, which is a property of CREATION, never of use
// =====================================================================================

/// A sealed `memfd` — memory a **second process** can map.
///
/// ## ★ Why this is a type rather than a descriptor parameter
///
/// `l1_os_shell.md` §4.4.1 and `Vmm::export_ram`'s rustdoc state the precondition the
/// whole isolate double-mmap rests on: *guest RAM is shareable with another process only
/// if the VM was **launched** with a shareable backing.* That is a **deployment fact no
/// code gate can observe** — on a hypervisor it is a command-line flag. The only
/// available mechanism is a loud refusal at the first export, and a refusal needs
/// something to be true or false *about*. This type is that something: an adapter either
/// holds one or it does not, and [`crate::Backing::PrivateAnonymous`] (the other arm)
/// documents the failure it prevents — copy-on-write pages, an isolate writing a
/// completion the guest never sees.
///
/// ## The seals, and the one that is load-bearing
///
/// `F_SEAL_SHRINK` is §11 item 4's requirement and it is not hygiene: a descriptor we
/// hand to a **sandboxed** isolate that could `ftruncate` it shorter turns every mapping
/// of it — ours included — into `SIGBUS` on the truncated pages, from the far side of the
/// #14 boundary. `F_SEAL_GROW` goes with it because a backing that can grow makes every
/// length we recorded a lower bound rather than a length, and `F_SEAL_SEAL` stops the
/// seals themselves being removed later.
///
/// **Deliberately NOT `F_SEAL_WRITE`:** the point of this memory is that both sides
/// write it.
#[derive(Debug)]
pub struct SharedRam {
    fd: OwnedFd,
    len: u64,
}

impl SharedRam {
    /// Create `len` bytes of shareable, sealed backing.
    ///
    /// # Errors
    /// [`RawError::ZeroLength`], [`RawError::TooLargeForHost`], [`RawError::Syscall`]
    /// (`memfd_create`, `ftruncate` or `fcntl`).
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create(len: u64) -> Result<Self, RawError> {
        Self::create_named(c"kayfabe-guest-ram", len)
    }

    /// Create `len` bytes of shareable, sealed backing under a **chosen** creation name.
    ///
    /// ## ★ Why the name is a parameter at all
    ///
    /// A `memfd`'s creation name is the only thing that distinguishes it from every other
    /// `memfd` in a process — it is what [`crate::MemfdCensus`] matches on, because a
    /// `memfd` has no path. [`SharedRam::create`]'s single fixed name is right for
    /// production (one kind of block, one name) and is *wrong for the census's own tests*:
    /// this crate's test binary runs in parallel threads, so two tests sharing a name is
    /// two blocks the census cannot tell apart — which is a real property of the census
    /// ([`crate::MemfdRefusal::Ambiguous`]) leaking into unrelated tests as flakiness.
    ///
    /// ⊘ It is `pub` rather than `pub(crate)` because the consumer that most needs it is
    /// **in another crate**: `kayfabe-qemu-raw`'s test for the guest-RAM crossing has to
    /// stand up a block carrying the hypervisor's own backend name, so that "the shim
    /// refuses when the descriptor is absent" can be controlled against "and accepts when
    /// it is present" — one process, one call, one thing changed. A production caller that
    /// wanted a differently-named block would be creating a second KIND of shared memory,
    /// which is a design change and not a parameter.
    ///
    /// # Errors
    /// As [`SharedRam::create`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create_named(name: &std::ffi::CStr, len: u64) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("memfd_create (a shareable guest-RAM backing)");
        if len == 0 {
            return Err(RawError::ZeroLength {
                what: "shared-RAM length",
            });
        }
        let size =
            libc::off_t::try_from(len).map_err(|_| RawError::TooLargeForHost { value: len })?;

        // SAFETY: `memfd_create` reads the NUL-terminated name and dereferences nothing
        // else; `name` is a `&CStr`, so the terminator is guaranteed by the type rather
        // than by us, and the borrow outlives the call. It returns a fresh descriptor or a
        // negative error, and `adopt_fd` (which checks the sign) is what takes ownership —
        // this block retains nothing.
        let raw = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_ALLOW_SEALING) };
        let fd = adopt_fd(raw, "memfd_create")?;

        // SAFETY: `fd` is a live descriptor this function owns (adopted on the line
        // above), `size` is a checked `off_t` conversion of a non-zero length, and
        // `ftruncate` dereferences no user memory at all.
        let rc = unsafe { libc::ftruncate(fd.as_raw_fd(), size) };
        if rc != 0 {
            return Err(last_syscall_error("ftruncate"));
        }

        // SAFETY: the same live descriptor; `F_ADD_SEALS` takes an integer bitmask by
        // value and dereferences no user memory. The seals are applied AFTER `ftruncate`,
        // which is the only order that works — `F_SEAL_SHRINK`/`F_SEAL_GROW` freeze the
        // size, so sealing first would freeze it at zero.
        let rc = unsafe {
            libc::fcntl(
                fd.as_raw_fd(),
                libc::F_ADD_SEALS,
                libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL,
            )
        };
        if rc != 0 {
            return Err(last_syscall_error("fcntl(F_ADD_SEALS)"));
        }
        Ok(SharedRam { fd, len })
    }

    /// The backing's length in bytes — frozen by the seals, so this is a length and not a
    /// snapshot of one.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.len
    }

    /// Borrow the descriptor, for mapping it ([`crate::Backing::SharedFile`]).
    #[must_use]
    pub fn as_backing_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Duplicate the descriptor, for handing to another process.
    ///
    /// The duplicate is close-on-exec: a descriptor that leaks across an unrelated `exec`
    /// is a share nobody declared, which is the same class of mistake as mapping the
    /// wrong range.
    ///
    /// # Errors
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn dup_for_export(&self) -> Result<OwnedFd, RawError> {
        lockwitness::assert_lock_free("fcntl(F_DUPFD_CLOEXEC) (exporting guest RAM)");
        // SAFETY: `self.fd` is a live descriptor owned by `self` for the duration of this
        // borrow; `F_DUPFD_CLOEXEC` takes an integer lower bound by value and
        // dereferences no user memory. The new descriptor is adopted by `adopt_fd`, which
        // checks the sign, so ownership is unique on both sides.
        let raw = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        adopt_fd(raw, "fcntl(F_DUPFD_CLOEXEC)")
    }
}

/// ★ How many descriptors this process may hold at once — the **real** soft limit, read
/// from the kernel at startup.
///
/// `l1_os_shell.md` §3.8 requires this and says why: one armed guest os-event costs a
/// descriptor in *our* process, so *"a hostile guest arming in a loop exhausts
/// `RLIMIT_NOFILE` in the VMM process — which is not a contained refusal, it is a
/// device-wide DoS (I4 violated) and quite possibly a VMM-wide one."* The core's cap is a
/// named constant; **this** is the reality it must not be set optimistically past.
///
/// Reports the *soft* limit, because that is the one a descriptor allocation actually
/// fails against. `RLIM_INFINITY` is reported as [`u64::MAX`] and is a real value on some
/// containers, so callers must reserve a fraction rather than subtract a constant.
///
/// # Errors
/// [`RawError::Syscall`].
///
/// # Panics
/// If called with any ranked lock held (R1, §4.5).
pub fn descriptor_budget() -> Result<u64, RawError> {
    lockwitness::assert_lock_free("getrlimit(RLIMIT_NOFILE)");
    leafwitness::assert_leaf_free("getrlimit(RLIMIT_NOFILE)");
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `lim` is a live `libc::rlimit` in this frame and `getrlimit` writes exactly
    // that struct through the exclusive borrow taken on this line; the resource argument
    // is an integer passed by value. The return value is checked below before any field
    // is read.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, core::ptr::from_mut(&mut lim)) };
    if rc < 0 {
        return Err(last_syscall_error("getrlimit"));
    }
    // `rlim_t` is 64-bit on the targets we build for and 32-bit on some others; the
    // conversion is therefore an identity here and a widening elsewhere, which is exactly
    // the case `useless_conversion` cannot see across a `cfg`.
    #[allow(clippy::useless_conversion)]
    Ok(u64::try_from(lim.rlim_cur).unwrap_or(u64::MAX))
}

// =====================================================================================
// Notifier — the ONE syscall this crate permits under a lock, and it is named so
// =====================================================================================

/// A counter-shaped notify descriptor: a signal that never blocks and never waits.
#[derive(Debug)]
pub struct Notifier {
    fd: OwnedFd,
}

impl Notifier {
    /// Create a non-blocking, close-on-exec notify descriptor.
    ///
    /// # Errors
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create() -> Result<Self, RawError> {
        lockwitness::assert_lock_free("eventfd (creating a notify descriptor)");
        leafwitness::assert_leaf_free("eventfd (creating a notify descriptor)");
        // SAFETY: this call takes two integers by value and dereferences no user memory.
        // `EFD_NONBLOCK` is what makes `signal_under_lock` below defensible: with it, the
        // only way the write can fail is the 64-bit counter saturating, which is a
        // refusal rather than a wait.
        let raw = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        Ok(Notifier {
            fd: adopt_fd(raw, "eventfd")?,
        })
    }

    /// Borrow the descriptor, for registering it in a readiness set (M2-d).
    #[must_use]
    pub fn as_source_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// ★★ **The named in-lock syscall** — `l1_os_shell.md` §4.5's escape hatch, and the
    /// only member of its set.
    ///
    /// §6.1 classifies `Vmm::raise_irq` as in-lock **legal**, on the grounds that it is
    /// *"one descriptor write on an irqfd"*. That legality is a claim about the
    /// **implementation**, not about the name, so it is spelled here where the syscall
    /// actually is: an 8-byte write to a non-blocking counter descriptor, which the
    /// kernel completes without sleeping, without allocating and without acquiring
    /// anything a peer thread can hold.
    ///
    /// `permitted_ranks` is a **mask of the lock ranks this call site is allowed to be
    /// holding**, named in the signature exactly as §4.5 requires, and *asserted* rather
    /// than documented: a caller that drifts into holding a rank it did not declare gets
    /// the same loud panic as a caller of an ordinary blocking door. Passing `0` makes it
    /// [`Notifier::signal`].
    ///
    /// # Errors
    /// [`RawError::Syscall`] — including `EAGAIN`, which for a non-blocking counter
    /// descriptor means it saturated, i.e. **nobody has drained this source in 2^64
    /// signals**. Reported, never retried in a loop: a retry loop is a blocking call
    /// wearing a non-blocking API.
    ///
    /// # Panics
    /// If this thread holds any rank outside `permitted_ranks`.
    pub fn signal_under_lock(&self, permitted_ranks: u8) -> Result<(), RawError> {
        lockwitness::assert_only_ranks(
            "a notify-descriptor write (the irqfd-shaped completion edge)",
            permitted_ranks,
        );
        let buf = 1u64.to_ne_bytes();
        // SAFETY: `buf` is a live 8-byte array in this frame and `write` reads exactly
        // `buf.len()` bytes from it — the length passed is computed here from the array
        // itself, so no caller supplies it. `self.fd` is live for this borrow. The return
        // value is checked below.
        let rc = unsafe {
            libc::write(
                self.fd.as_raw_fd(),
                buf.as_ptr().cast::<libc::c_void>(),
                buf.len(),
            )
        };
        if rc != buf.len() as isize {
            return Err(last_syscall_error("write(notify descriptor)"));
        }
        Ok(())
    }

    /// Signal with **no** lock held — the ordinary door.
    ///
    /// # Errors
    /// As [`Notifier::signal_under_lock`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn signal(&self) -> Result<(), RawError> {
        self.signal_under_lock(0)
    }

    /// Drain the counter, returning how many signals had accumulated (0 if none).
    ///
    /// ★ This is what makes level-triggered readiness self-clearing (§3.4(c)): the read
    /// resets the counter, so the source stops being ready. A reactor that skips it spins.
    ///
    /// # Errors
    /// [`RawError::Syscall`] for anything other than "empty", which is `Ok(0)`.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn drain(&self) -> Result<u64, RawError> {
        lockwitness::assert_lock_free("a notify-descriptor read (draining a source)");
        leafwitness::assert_leaf_free("a notify-descriptor read (draining a source)");
        let mut buf = [0u8; 8];
        // SAFETY: `buf` is a live 8-byte array in this frame and `read` writes at most
        // `buf.len()` bytes into it — the length is computed here from the array itself.
        // `self.fd` is live for this borrow.
        let rc = unsafe {
            libc::read(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr().cast::<libc::c_void>(),
                buf.len(),
            )
        };
        if rc == buf.len() as isize {
            return Ok(u64::from_ne_bytes(buf));
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(e) if e == libc::EAGAIN => Ok(0),
            _ => Err(last_syscall_error("read(notify descriptor)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backing, CachePolicy, HostOffset, HostPageSize, HostProt, MappedRegion};

    /// A named failing syscall, re-runnable: the name is for the assertion message, the
    /// closure re-provokes the failure (and with it the `errno`) at the moment it is
    /// wanted.
    type FailingSyscall = (&'static str, Box<dyn Fn() -> std::io::Error>);

    /// Four different kernel `errno`s, each provoked by a **real** failing syscall reached
    /// through safe std, so the value under test is the kernel's and not one we typed.
    ///
    /// Each closure leaves `errno` set to its own value, and nothing between the call and
    /// [`adopt_fd`] issues another syscall — which is the whole mechanism
    /// [`last_syscall_error`] relies on in production, exercised here rather than assumed.
    fn failing_syscalls() -> Vec<FailingSyscall> {
        let too_long = format!("/{}", "n".repeat(5000));
        vec![
            (
                "a path that is not there",
                Box::new(|| {
                    std::fs::File::open("/kayfabe-no-such-path-4a1f").expect_err("must fail")
                }) as Box<dyn Fn() -> std::io::Error>,
            ),
            (
                "a non-directory in the middle of a path",
                Box::new(|| std::fs::File::open("/etc/hostname/child").expect_err("must fail")),
            ),
            (
                "a path longer than the kernel accepts",
                Box::new(move || std::fs::File::open(&too_long).expect_err("must fail")),
            ),
            (
                "opening a directory for writing",
                Box::new(|| {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open("/")
                        .expect_err("must fail")
                }),
            ),
        ]
    }

    /// ★★ **The one adoption site's refusal**, swept over every shape a failed syscall's
    /// return takes and every `errno` four real failures produce.
    ///
    /// [`adopt_fd`] guards the only `from_raw_fd` in the crate. A guard that let a
    /// negative through would hand `OwnedFd` a **negative `errno` as a descriptor** — an
    /// owned thing that is not a descriptor, closed on drop, and thereafter aliasable by
    /// whatever integer the next failure produces. Nothing downstream can notice, because
    /// every caller of this function has already discarded the raw value.
    ///
    /// Swept deliberately: `-1` (the C convention), `-errno` (what a raw syscall returns),
    /// and both ends of the range — a guard tested at one value is a guard tested at one
    /// value.
    #[test]
    fn a_negative_syscall_return_is_refused_by_exact_errno_and_adopts_nothing() {
        let mut seen = Vec::new();
        for (what, provoke) in failing_syscalls() {
            let errno = provoke().raw_os_error();
            assert!(
                errno.is_some(),
                "{what} must have failed with a real errno for this case to test anything"
            );
            seen.push(errno);
            for raw in [
                -1,
                -2,
                -libc::EMFILE,
                -libc::EBADF,
                -libc::ENOMEM,
                libc::c_int::MIN,
                libc::c_int::MIN + 1,
            ] {
                assert_eq!(
                    adopt_fd(raw, "a swept adoption").unwrap_err(),
                    RawError::Syscall {
                        call: "a swept adoption",
                        errno,
                    },
                    "{what}: a return of {raw} must be refused as the errno the kernel \
                     set, and must NOT become an owned descriptor"
                );
            }
        }
        // The instrument, checked: four provokers that all landed on the SAME errno would
        // make the sweep above one case repeated four times and nothing would say so.
        let distinct = seen.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            distinct.len(),
            seen.len(),
            "the provokers must produce DIFFERENT errnos, or this sweep is one case wearing \
             four names: {seen:?}"
        );
    }

    /// ★ **Zero is a descriptor.** The counterpart to the sweep above, and not a formality:
    /// a guard written `<= 0` refuses `stdin`, and one written `== 0` refuses only zero
    /// while adopting every negative — the two mistakes disagree about exactly this value,
    /// so only a case *at* zero tells them apart.
    ///
    /// A stale `errno` is left set on purpose: a valid descriptor must be adopted on the
    /// strength of its own sign, never on whatever the last failing call left behind.
    ///
    /// `into_raw_fd` gives the number back **without closing it** — these are the test
    /// process's own standard descriptors and integers it does not own; closing them would
    /// break the harness rather than the code under test.
    #[test]
    fn descriptor_zero_and_every_other_non_negative_return_is_adopted_unchanged() {
        use std::os::fd::IntoRawFd as _;

        let _ = std::fs::File::open("/kayfabe-no-such-path-4a1f").expect_err("must fail");
        for raw in [0, 1, 2, 3, 42, libc::c_int::MAX] {
            let adopted = adopt_fd(raw, "a swept adoption").unwrap_or_else(|e| {
                panic!(
                    "a return of {raw} is a DESCRIPTOR the kernel just handed us, not a \
                     failure; refusing it leaks the descriptor forever and turns a \
                     working call into an error ({e:?})"
                )
            });
            assert_eq!(
                adopted.as_raw_fd(),
                raw,
                "adoption must preserve the number the kernel returned"
            );
            assert_eq!(adopted.into_raw_fd(), raw);
        }
    }

    /// The seal is the point, so it is proven by the KERNEL refusing, not by us
    /// remembering to ask. `File::set_len` is `ftruncate`, reached through safe std.
    #[test]
    fn a_shared_backing_is_sealed_against_shrinking() {
        let ram = SharedRam::create(2 * 4096).expect("memfd_create is available on Linux");
        assert_eq!(ram.len_bytes(), 2 * 4096);
        let f = std::fs::File::from(ram.dup_for_export().expect("dup"));
        let err = f
            .set_len(4096)
            .expect_err("a sealed backing must refuse to shrink");
        assert_eq!(
            err.raw_os_error(),
            Some(libc::EPERM),
            "F_SEAL_SHRINK must make a shorter ftruncate EPERM — a backing a sandboxed \
             isolate can shrink turns every mapping of it into SIGBUS from the far side \
             of the #14 boundary (§11 item 4)"
        );
    }

    #[test]
    fn a_zero_length_shared_backing_is_refused_by_that_exact_variant() {
        assert_eq!(
            SharedRam::create(0).unwrap_err(),
            RawError::ZeroLength {
                what: "shared-RAM length"
            }
        );
    }

    /// ★ The property §4.4.1 rests on, asserted rather than assumed: a duplicated
    /// descriptor is the SAME backing, not a copy-on-write snapshot of one.
    #[test]
    fn a_duplicate_of_a_shared_backing_maps_the_same_bytes() {
        let page = HostPageSize::query();
        let ram = SharedRam::create(page.bytes()).expect("memfd");
        let dup = ram.dup_for_export().expect("dup");
        let a = MappedRegion::map(
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: 0,
            },
            page.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .expect("map the original");
        let b = MappedRegion::map(
            Backing::SharedFile {
                fd: dup.as_fd(),
                offset: 0,
            },
            page.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .expect("map the duplicate");
        a.write_from(HostOffset::ZERO, &[0xA5; 8])
            .expect("write through the original");
        let mut got = [0u8; 8];
        b.read_into(HostOffset::ZERO, &mut got)
            .expect("read through the duplicate");
        assert_eq!(
            got, [0xA5; 8],
            "a duplicated shareable backing must be the SAME memory — were it \
             copy-on-write, an isolate's completions would be invisible to the guest, \
             which is the exact failure §4.4.1 exists to prevent"
        );
    }

    #[test]
    fn a_notify_signal_is_counted_and_drained_exactly_once() {
        let n = Notifier::create().expect("a notify descriptor");
        assert_eq!(n.drain(), Ok(0), "a fresh notifier has nothing pending");
        n.signal().expect("signal");
        n.signal().expect("signal");
        assert_eq!(
            n.drain(),
            Ok(2),
            "the source COALESCES: two signals drain as one edge carrying the count"
        );
        assert_eq!(n.drain(), Ok(0), "and the counter is consumed by the drain");
    }

    /// ★ The escape hatch's polarity, both ways. This is the only syscall in the
    /// workspace permitted under a lock; a rank it did not declare must still panic, or
    /// the hatch is a hole.
    #[test]
    fn the_named_in_lock_syscall_permits_only_the_ranks_it_declares() {
        let n = Notifier::create().expect("a notify descriptor");
        lockwitness::note_acquired(0);
        let permitted = n.signal_under_lock(lockwitness::bit(0));
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            n.signal_under_lock(lockwitness::bit(1))
        }));
        lockwitness::note_released(0);
        assert_eq!(
            permitted,
            Ok(()),
            "holding EXACTLY the declared rank must be permitted"
        );
        assert!(
            refused.is_err(),
            "holding rank 0 while declaring only rank 1 must panic — an under-lock \
             variant that accepts any lock is not an exception, it is an exemption"
        );
    }
}
