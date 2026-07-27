//! The readiness set — `l1_os_shell.md` §3, the OS half of the completion-source reactor.
//!
//! ## ★ Level-triggered, and the key that is not a descriptor (§3.2, §3.4)
//!
//! Two design decisions are baked into this type's shape, and both are the doc's:
//!
//! - **Level-triggered** (§3.4 option (c)). Not edge, not one-shot. *"Every source the
//!   reactor watches is a counter we own, the loop reads it (which resets it to zero), and
//!   readiness therefore self-clears. No re-arm to forget, no edge to miss, no spin."*
//!   That makes the drain the loop's obligation rather than the kernel's — which is why
//!   the loop above this type counts what it drained and refuses loudly when a ready
//!   report drains nothing.
//! - **The token is caller-chosen, 64 bits wide, and is NOT the descriptor** (§3.2).
//!   A descriptor number can be recycled by the kernel the instant it is closed; a
//!   `CompletionSource` never can. Keying readiness on the descriptor would let a *new*
//!   source inherit a *stale* readiness report — the C's F4 use-after-retire, rebuilt in
//!   the adapter. So [`Poller::watch`] takes an opaque `token` and [`ReadyTokens`] hands
//!   back exactly that, with no way to recover the descriptor from it.
//!
//! ## What does not cross this seam
//!
//! No descriptor. [`Poller::watch`] borrows one for the duration of the call and keeps
//! nothing; ownership stays with whoever created the source, and the §3.3 rule
//! (*"deregister then close, never close then deregister"*) is therefore expressible —
//! and is the caller's, because only the caller knows when the last reference goes away.

use crate::error::{RawError, last_syscall_error};
use crate::host_fd_unsafe::adopt_fd;
use kayfabe_util::{leafwitness, lockwitness};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};

/// The largest batch one [`Poller::wait`] may report.
///
/// A bound rather than a `Vec` the kernel sizes: the buffer is stack-shaped, reused across
/// iterations, and a kernel that had more to say simply says it on the next wait — which
/// costs one extra loop iteration and cannot cost an allocation on the readiness path.
pub const MAX_READY_BATCH: usize = 64;

/// How long a [`Poller::wait`] may block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollTimeout {
    /// Return immediately, ready or not.
    Immediate,
    /// Block for at most this many milliseconds.
    Millis(u32),
    /// Block until something is ready. The reactor loop's normal mode: it is woken by
    /// its own notify source for shutdown and injected work (§3.6), so an indefinite
    /// wait is not a liveness hazard — it is the absence of a poll.
    Blocking,
}

impl PollTimeout {
    fn as_millis(self) -> libc::c_int {
        match self {
            PollTimeout::Immediate => 0,
            // Saturating rather than wrapping: a caller asking for a longer wait than the
            // kernel's argument can express gets the longest expressible wait, never a
            // spin (which is what a wrapped negative-to-zero would produce).
            PollTimeout::Millis(ms) => libc::c_int::try_from(ms).unwrap_or(libc::c_int::MAX),
            PollTimeout::Blocking => -1,
        }
    }
}

/// The tokens one [`Poller::wait`] reported, in kernel order.
///
/// Deliberately a **bounded owned buffer**, not a borrow into kernel-filled memory: the
/// events array is written by `epoll_wait` and read once, and copying the tokens out at
/// the syscall boundary means nothing above this crate ever holds a reference into it.
#[derive(Debug)]
pub struct ReadyTokens {
    tokens: [u64; MAX_READY_BATCH],
    len: usize,
}

impl Default for ReadyTokens {
    fn default() -> Self {
        ReadyTokens {
            tokens: [0; MAX_READY_BATCH],
            len: 0,
        }
    }
}

impl ReadyTokens {
    /// An empty batch, ready to be filled by [`Poller::wait`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many tokens the last wait reported.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True if the last wait reported nothing (a timeout).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The reported tokens, in the order the kernel gave them.
    pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.tokens[..self.len].iter().copied()
    }
}

/// A readiness set: the one descriptor the reactor loop blocks on.
#[derive(Debug)]
pub struct Poller {
    fd: OwnedFd,
}

impl Poller {
    /// Create an empty readiness set.
    ///
    /// # Errors
    /// [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create() -> Result<Self, RawError> {
        lockwitness::assert_lock_free("creating a readiness set");
        leafwitness::assert_leaf_free("creating a readiness set");
        // SAFETY: this call takes one integer flag by value and dereferences no user
        // memory. The result is adopted by `adopt_fd`, which checks the sign, so ownership
        // is unique and the descriptor is closed exactly once.
        let raw = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        Ok(Poller {
            fd: adopt_fd(raw, "epoll_create1")?,
        })
    }

    /// Watch `source` **level-triggered** for readability, reporting `token`.
    ///
    /// `token` is opaque here and is echoed verbatim by [`Poller::wait`]. It is the
    /// caller's identity for the source (§3.2: a `CompletionSource`, never a descriptor
    /// number).
    ///
    /// # Errors
    /// [`RawError::Syscall`] — `EEXIST` if this descriptor is already watched, `EPERM`
    /// for a descriptor that cannot be polled at all (a regular file).
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn watch(&self, source: BorrowedFd<'_>, token: u64) -> Result<(), RawError> {
        lockwitness::assert_lock_free("adding a source to the readiness set");
        leafwitness::assert_leaf_free("adding a source to the readiness set");
        // ★ No `EPOLLET`, no `EPOLLONESHOT`: level-triggered is the design (§3.4(c)) and
        // the drain is what clears it. `EPOLLHUP`/`EPOLLERR` are reported whether or not
        // they are requested, which is what makes a peer-closed channel a readiness
        // report rather than a silence.
        let readable = u32::try_from(libc::EPOLLIN).expect("EPOLLIN is a small positive flag");
        self.ctl(libc::EPOLL_CTL_ADD, source, readable, token)
    }

    /// Stop watching `source`.
    ///
    /// ★ §3.3's rule lives with the caller, not here: **deregister then close, never close
    /// then deregister.** Closing a descriptor removes it from the set only if it was the
    /// last reference; a duplicated one silently stays and keeps firing.
    ///
    /// # Errors
    /// [`RawError::Syscall`] — `ENOENT` if it was not watched.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn unwatch(&self, source: BorrowedFd<'_>) -> Result<(), RawError> {
        lockwitness::assert_lock_free("removing a source from the readiness set");
        leafwitness::assert_leaf_free("removing a source from the readiness set");
        self.ctl(libc::EPOLL_CTL_DEL, source, 0, 0)
    }

    /// Wait for readiness, filling `out` with the ready tokens.
    ///
    /// Returns how many tokens were reported; `0` is a timeout, and — for
    /// [`PollTimeout::Blocking`] — an interrupted wait (`EINTR`), which is reported as an
    /// empty batch rather than an error because the loop's answer to both is identical:
    /// go round again.
    ///
    /// # Errors
    /// [`RawError::Syscall`] for anything other than `EINTR`.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5) — this is the single most blocking
    /// call in the system.
    pub fn wait(&self, out: &mut ReadyTokens, timeout: PollTimeout) -> Result<usize, RawError> {
        lockwitness::assert_lock_free("waiting on the readiness set");
        leafwitness::assert_leaf_free("waiting on the readiness set");
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_READY_BATCH];
        // SAFETY: `events` is a live array of exactly `MAX_READY_BATCH`
        // `libc::epoll_event`s in this frame, and the count passed is derived from that
        // same array's length on this line — no caller supplies it — so the kernel writes
        // at most the bytes this borrow covers. `self.fd` is live for this borrow. The
        // return value is checked below before any element is read.
        let rc = unsafe {
            libc::epoll_wait(
                self.fd.as_raw_fd(),
                events.as_mut_ptr(),
                libc::c_int::try_from(MAX_READY_BATCH).unwrap_or(libc::c_int::MAX),
                timeout.as_millis(),
            )
        };
        if rc < 0 {
            return match std::io::Error::last_os_error().raw_os_error() {
                Some(e) if e == libc::EINTR => {
                    out.len = 0;
                    Ok(0)
                }
                _ => Err(last_syscall_error("epoll_wait")),
            };
        }
        let n = (rc as usize).min(MAX_READY_BATCH);
        for (slot, ev) in out.tokens[..n].iter_mut().zip(events[..n].iter()) {
            *slot = ev.u64;
        }
        out.len = n;
        Ok(n)
    }

    fn ctl(
        &self,
        op: libc::c_int,
        source: BorrowedFd<'_>,
        events: u32,
        token: u64,
    ) -> Result<(), RawError> {
        let mut ev = libc::epoll_event { events, u64: token };
        // SAFETY: `ev` is a live `libc::epoll_event` in this frame for the duration of the
        // call; the kernel reads it (and, for `EPOLL_CTL_DEL`, ignores it) and never
        // retains it. `self.fd` and `source` are both borrowed from live descriptors by
        // the caller in this same expression. The return value is checked below.
        let rc = unsafe {
            libc::epoll_ctl(
                self.fd.as_raw_fd(),
                op,
                source.as_raw_fd(),
                core::ptr::from_mut(&mut ev),
            )
        };
        if rc < 0 {
            return Err(last_syscall_error("epoll_ctl"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Notifier;
    use std::os::fd::AsFd;

    /// The token is the caller's identity, echoed verbatim — never the descriptor.
    /// Asserted with a token that is deliberately **not** a plausible descriptor number,
    /// so an implementation that reported `ev.data.fd` could not accidentally match.
    #[test]
    fn the_reported_token_is_the_callers_and_never_the_descriptor() {
        let p = Poller::create().expect("readiness set");
        let n = Notifier::create().expect("counter");
        const TOKEN: u64 = 0xDEAD_BEEF_0000_002A;
        p.watch(n.as_source_fd(), TOKEN).expect("watch");
        n.signal().expect("signal");

        let mut ready = ReadyTokens::new();
        assert_eq!(
            p.wait(&mut ready, PollTimeout::Millis(500)).expect("wait"),
            1
        );
        assert_eq!(ready.iter().collect::<Vec<_>>(), vec![TOKEN]);
    }

    /// ★ **Level-triggered, and the drain is what clears it.** The same counter reports
    /// ready again and again until it is read — which is precisely why the loop above
    /// this type must drain, and why an undrainable source is a spin. Both halves are
    /// pinned: still-ready before the drain, quiet after it.
    #[test]
    fn readiness_is_level_triggered_and_self_clears_only_on_the_drain() {
        let p = Poller::create().expect("readiness set");
        let n = Notifier::create().expect("counter");
        p.watch(n.as_source_fd(), 7).expect("watch");
        n.signal().expect("signal");
        n.signal().expect("signal");

        let mut ready = ReadyTokens::new();
        // Two waits with NO drain in between: level-triggered reports it both times.
        for round in 0..2 {
            assert_eq!(
                p.wait(&mut ready, PollTimeout::Immediate).expect("wait"),
                1,
                "round {round}: a level-triggered source stays ready until it is drained"
            );
        }
        // The drain coalesces: two signals, one read, the count is two.
        assert_eq!(n.drain().expect("drain"), 2);
        assert_eq!(
            p.wait(&mut ready, PollTimeout::Immediate).expect("wait"),
            0,
            "the drain, and only the drain, clears the readiness"
        );
        assert!(ready.is_empty());
    }

    /// Unwatching really removes it, and a timeout is an empty batch rather than an
    /// error — the two halves the loop's shutdown path depends on.
    #[test]
    fn unwatch_removes_the_source_and_a_timeout_is_an_empty_batch() {
        let p = Poller::create().expect("readiness set");
        let n = Notifier::create().expect("counter");
        p.watch(n.as_source_fd(), 1).expect("watch");
        n.signal().expect("signal");
        p.unwatch(n.as_source_fd()).expect("unwatch");

        let mut ready = ReadyTokens::new();
        assert_eq!(
            p.wait(&mut ready, PollTimeout::Immediate).expect("wait"),
            0,
            "an unwatched source is silent even while it is readable"
        );
        // ...and unwatching twice is the kernel's ENOENT, not our opinion.
        assert_eq!(
            p.unwatch(n.as_source_fd()).unwrap_err(),
            RawError::Syscall {
                call: "epoll_ctl",
                errno: Some(libc::ENOENT),
            }
        );
    }

    /// ★ A peer-closed pipe is **permanently readable and drains nothing** — the exact
    /// hazard §3.4(c)'s "every source is a counter we drain" does not cover, pinned here
    /// at the layer that can see it. `SourceKind::Worker` is this shape (§3.1: readiness
    /// means HUP), so the loop must treat it as terminal and unwatch it, not drain it.
    #[test]
    fn a_hung_up_channel_is_readable_forever_and_yields_nothing_to_read() {
        let (reader, writer) = std::io::pipe().expect("a control channel");
        let p = Poller::create().expect("readiness set");
        p.watch(reader.as_fd(), 0x5152).expect("watch");

        let mut ready = ReadyTokens::new();
        assert_eq!(
            p.wait(&mut ready, PollTimeout::Immediate).expect("wait"),
            0,
            "a live channel with nothing on it is not ready"
        );
        drop(writer); // the worker dies

        for round in 0..3 {
            assert_eq!(
                p.wait(&mut ready, PollTimeout::Immediate).expect("wait"),
                1,
                "round {round}: HUP is level-triggered and there is nothing to read that \
                 would clear it — a loop that only knows how to drain spins here forever"
            );
            assert_eq!(ready.iter().collect::<Vec<_>>(), vec![0x5152]);
        }
    }
}
