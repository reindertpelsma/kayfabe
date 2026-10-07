// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ A non-blocking `AF_UNIX` stream **client** — the display broker's socket
//! (`docs/design/V3_DISPLAY.md` §8, the broker relay).
//!
//! Five doors, each one syscall wide, and nothing that decides anything:
//!
//! - [`check_socket_path`] — the path rules, checked before any syscall: absolute, shorter than
//!   `sun_path`, never the abstract namespace (an abstract socket has no filesystem permissions,
//!   so anyone in the network namespace could squat the name).
//! - [`unix_connect`] — `socket(SOCK_NONBLOCK)` + `connect`. ⊘ **There is no "in progress"
//!   state for `AF_UNIX`**: a stream `connect` succeeds or fails at once, and `EAGAIN` means the
//!   listener's backlog is full with nothing pending (linux `net/unix/af_unix.c:1698-1701`). So
//!   every error, `EAGAIN` included, is returned as a failed attempt; a caller that treated it
//!   as `EINPROGRESS` (a TCP habit) would poll an unconnected socket until its own deadline.
//! - [`peer_credentials`] — `SO_PEERCRED`. The kernel copies the listener's credentials at
//!   `connect` (`af_unix.c:1774-1776`), so they exist before the first byte and cannot be
//!   forged by the peer.
//! - [`send_record`] — `sendmsg(MSG_NOSIGNAL | MSG_DONTWAIT)` of one record, with at most ONE
//!   descriptor as `SCM_RIGHTS`. It reports how many bytes went; a short record is the
//!   caller's protocol error, never resent here (a resent prefix would re-send the descriptor).
//! - [`recv_bounded`] — `recvmsg(MSG_DONTWAIT)` with **no control buffer**. The kernel then
//!   drops any descriptor the peer attached and sets `MSG_CTRUNC` (`net/core/scm.c:501-511`),
//!   which this door reports so the caller can treat it as the protocol violation it is. A peer
//!   cannot make this process hold a descriptor it never asked for.
//!
//! Every door asserts both witnesses, as every syscall in this crate does.

use crate::error::{RawError, last_syscall_error};
use crate::host_fd_unsafe::adopt_fd;
use kf_util::{leafwitness, lockwitness};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `sizeof(sockaddr_un.sun_path)` on Linux. A path must be strictly shorter (room for the NUL).
pub const SUN_PATH_BYTES: usize = 108;

/// The credentials the kernel recorded for the peer of a connected socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    /// The peer's process id at `connect`/`listen` time.
    pub pid: i32,
    /// The peer's effective uid.
    pub uid: u32,
    /// The peer's effective gid.
    pub gid: u32,
}

/// What one [`recv_bounded`] read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Received {
    /// Bytes written into the caller's buffer; `0` is end of stream.
    pub bytes: usize,
    /// ★ The peer attached a descriptor (or other control data) and the kernel dropped it
    /// (`MSG_CTRUNC`). A protocol whose peer never sends descriptors treats this as fatal.
    pub control_dropped: bool,
}

/// ★ The path rules for a socket this process will CONNECT to, checked without a syscall.
///
/// # Errors
/// [`RawError::BadSocketPath`] naming the rule: empty, relative (which includes the `@name`
/// spelling some tools use for the abstract namespace), an embedded NUL (the abstract namespace
/// itself), or too long for `sun_path`.
pub fn check_socket_path(path: &Path) -> Result<(), RawError> {
    let b = path.as_os_str().as_bytes();
    if b.is_empty() {
        return Err(RawError::BadSocketPath {
            why: "the path is empty",
        });
    }
    if b.contains(&0) {
        return Err(RawError::BadSocketPath {
            why: "the path contains a NUL byte (an abstract-namespace name has no filesystem \
                  permissions, so anyone could listen on it)",
        });
    }
    if b[0] != b'/' {
        return Err(RawError::BadSocketPath {
            why: "the path is not absolute (relative paths resolve against QEMU's working \
                  directory, and `@name` would be an abstract-namespace name)",
        });
    }
    if b.len() >= SUN_PATH_BYTES {
        return Err(RawError::BadSocketPath {
            why: "the path does not fit sockaddr_un.sun_path (107 bytes at most)",
        });
    }
    Ok(())
}

/// ★ Connect a non-blocking, close-on-exec `AF_UNIX` stream socket to `path`.
///
/// # Errors
/// [`RawError::BadSocketPath`] (see [`check_socket_path`]); [`RawError::Syscall`] for `socket`
/// or `connect` — **including `EAGAIN`** (backlog full), which is a failed attempt and never
/// "in progress".
///
/// # Panics
/// If called with any ranked lock held, or from a leaf context.
pub fn unix_connect(path: &Path) -> Result<OwnedFd, RawError> {
    lockwitness::assert_lock_free("connect (an AF_UNIX client socket)");
    leafwitness::assert_leaf_free("connect (an AF_UNIX client socket)");
    check_socket_path(path)?;
    // SAFETY: three integers by value; no memory is dereferenced. The descriptor (or the
    // negative error) is consumed by `adopt_fd` on the same expression.
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    let sock = adopt_fd(raw, "socket(AF_UNIX)")?;
    let mut addr = libc::sockaddr_un {
        sun_family: libc::AF_UNIX as libc::sa_family_t,
        sun_path: [0; SUN_PATH_BYTES],
    };
    // `check_socket_path` bounded the length below SUN_PATH_BYTES, so the copy leaves the NUL.
    for (dst, src) in addr
        .sun_path
        .iter_mut()
        .zip(path.as_os_str().as_bytes().iter())
    {
        *dst = *src as libc::c_char;
    }
    // SAFETY: `addr` is a live, fully initialised `sockaddr_un` in this frame and the length
    // passed is its own `size_of`, so `connect` reads exactly that object and nothing else.
    // `sock` is a live descriptor this function owns. The return value is checked below.
    let rc = unsafe {
        libc::connect(
            sock.as_raw_fd(),
            core::ptr::from_ref(&addr).cast::<libc::sockaddr>(),
            core::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(last_syscall_error("connect(AF_UNIX)"));
    }
    Ok(sock)
}

/// ★ `SO_PEERCRED` of a connected `AF_UNIX` socket.
///
/// # Errors
/// [`RawError::Syscall`]; a reply of the wrong size is reported as one too (`errno` `None`).
///
/// # Panics
/// If called with any ranked lock held, or from a leaf context.
pub fn peer_credentials(sock: BorrowedFd<'_>) -> Result<PeerCredentials, RawError> {
    lockwitness::assert_lock_free("getsockopt(SO_PEERCRED)");
    leafwitness::assert_leaf_free("getsockopt(SO_PEERCRED)");
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = core::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are live locals in this frame; `getsockopt` writes at most
    // `len` (= the object's own size) bytes into `cred` and the actual size into `len`.
    let rc = unsafe {
        libc::getsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            core::ptr::from_mut(&mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(last_syscall_error("getsockopt(SO_PEERCRED)"));
    }
    if len as usize != core::mem::size_of::<libc::ucred>() {
        return Err(RawError::Syscall {
            call: "getsockopt(SO_PEERCRED) returned a short credential",
            errno: None,
        });
    }
    Ok(PeerCredentials {
        pid: cred.pid,
        uid: cred.uid,
        gid: cred.gid,
    })
}

/// ★ This process's effective uid, NOW — the other half of a [`peer_credentials`] check.
///
/// Read at each check, never cached: QEMU drops privileges (`-run-with user=`, `-runas`) in
/// `os_setup_post`, after every device is realized, so a value read at realize names root
/// while QEMU then runs as the dropped uid. `geteuid` cannot fail and, unlike
/// `/proc/self/status`, works inside a `-run-with chroot=` that has no `/proc`.
#[must_use]
pub fn effective_uid() -> u32 {
    // SAFETY: no arguments, no memory touched; `geteuid` always succeeds (POSIX). glibc applies
    // a `setuid` to every thread of the process, so the calling thread's answer is the process's.
    unsafe { libc::geteuid() }
}

/// A zeroed `msghdr`: on some libc targets it carries private padding fields, so it cannot be
/// written as a struct literal portably.
fn zeroed_msghdr() -> libc::msghdr {
    // SAFETY: `msghdr` is a plain C struct of pointers and integers; all-zero is its documented
    // empty value (null iov/control, zero lengths and flags).
    unsafe { core::mem::zeroed() }
}

/// Room for one `SCM_RIGHTS` header carrying one `int`, 8-byte aligned.
const ONE_FD_CMSG_U64S: usize = 3;

/// ★ Send one record — `body` in ONE `sendmsg`, with `fd` (when given) as a single
/// `SCM_RIGHTS` descriptor — without blocking and without `SIGPIPE`.
///
/// Returns the bytes the kernel took. A stream socket may take a prefix; the descriptor then
/// went with the first byte, so the caller must treat a short count as fatal rather than
/// resend (a resend would attach the descriptor twice).
///
/// # Errors
/// [`RawError::ZeroLength`] for an empty body; [`RawError::Syscall`] carrying `errno` — `EAGAIN`
/// ([`RawError::is_would_block`]) when the socket buffer is full.
///
/// # Panics
/// If called with any ranked lock held, or from a leaf context.
pub fn send_record(
    sock: BorrowedFd<'_>,
    body: &[u8],
    fd: Option<BorrowedFd<'_>>,
) -> Result<usize, RawError> {
    lockwitness::assert_lock_free("sendmsg (one broker record)");
    leafwitness::assert_leaf_free("sendmsg (one broker record)");
    if body.is_empty() {
        return Err(RawError::ZeroLength {
            what: "a socket record",
        });
    }
    let mut iov = libc::iovec {
        iov_base: body.as_ptr().cast::<libc::c_void>().cast_mut(),
        iov_len: body.len(),
    };
    let mut cmsg = [0u64; ONE_FD_CMSG_U64S];
    let mut msg = zeroed_msghdr();
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    if let Some(fd) = fd {
        let raw: libc::c_int = fd.as_raw_fd();
        // SAFETY: `cmsg` is a live, 8-byte-aligned `[u64; 3]` in this frame (u64 alignment is
        // `cmsghdr`'s on every target this crate builds for). `CMSG_SPACE(sizeof(int))` is
        // asserted to fit it before `msg_controllen` is set from it, so `CMSG_FIRSTHDR`
        // returns a pointer inside `cmsg` (asserted non-null), and `CMSG_DATA` of that header
        // has room for exactly the one `int` copied from `raw`, a live local.
        unsafe {
            let space = libc::CMSG_SPACE(core::mem::size_of::<libc::c_int>() as libc::c_uint);
            assert!(
                space as usize <= core::mem::size_of_val(&cmsg),
                "the control buffer is sized for one descriptor"
            );
            msg.msg_control = cmsg.as_mut_ptr().cast::<libc::c_void>();
            msg.msg_controllen = space as _;
            let hdr = libc::CMSG_FIRSTHDR(&raw const msg);
            assert!(!hdr.is_null(), "a sized control buffer has a first header");
            (*hdr).cmsg_level = libc::SOL_SOCKET;
            (*hdr).cmsg_type = libc::SCM_RIGHTS;
            (*hdr).cmsg_len =
                libc::CMSG_LEN(core::mem::size_of::<libc::c_int>() as libc::c_uint) as _;
            core::ptr::copy_nonoverlapping(
                (&raw const raw).cast::<u8>(),
                libc::CMSG_DATA(hdr),
                core::mem::size_of::<libc::c_int>(),
            );
        }
    }
    // SAFETY: `msg` is initialised above: `msg_iov` points at `iov` (live, describing `body`,
    // which outlives the call); the control fields are zero or point at `cmsg` (live) with a
    // length asserted to fit it. `sendmsg` only reads through them. Checked below.
    let sent = unsafe {
        libc::sendmsg(
            sock.as_raw_fd(),
            &raw const msg,
            libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
        )
    };
    if sent < 0 {
        return Err(last_syscall_error("sendmsg(broker record)"));
    }
    Ok(sent as usize)
}

/// ★ Read at most `buf.len()` bytes without blocking, accepting **no** descriptors.
///
/// # Errors
/// [`RawError::ZeroLength`] for an empty buffer; [`RawError::Syscall`] carrying `errno` —
/// `EAGAIN` ([`RawError::is_would_block`]) when nothing is waiting.
///
/// # Panics
/// If called with any ranked lock held, or from a leaf context.
pub fn recv_bounded(sock: BorrowedFd<'_>, buf: &mut [u8]) -> Result<Received, RawError> {
    lockwitness::assert_lock_free("recvmsg (broker events)");
    leafwitness::assert_leaf_free("recvmsg (broker events)");
    if buf.is_empty() {
        return Err(RawError::ZeroLength {
            what: "a receive buffer",
        });
    }
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: buf.len(),
    };
    let mut msg = zeroed_msghdr();
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    // SAFETY: `msg_iov` points at `iov`, live in this frame, describing `buf` (exclusively
    // borrowed for the call); `recvmsg` writes at most `buf.len()` bytes into it. There is no
    // control buffer, so nothing else is written except `msg.msg_flags`. Checked below.
    let got = unsafe { libc::recvmsg(sock.as_raw_fd(), &raw mut msg, libc::MSG_DONTWAIT) };
    if got < 0 {
        return Err(last_syscall_error("recvmsg(broker events)"));
    }
    Ok(Received {
        bytes: got as usize,
        control_dropped: msg.msg_flags & libc::MSG_CTRUNC != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::os::fd::AsFd;
    use std::os::unix::net::{UnixListener, UnixStream};

    fn scratch(tag: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("kfu-{tag}-{}-{nonce}.sock", std::process::id()))
    }

    #[test]
    fn relative_abstract_empty_and_overlong_paths_are_refused_by_name() {
        for (p, frag) in [
            ("", "empty"),
            ("relative.sock", "not absolute"),
            ("@abstract", "not absolute"),
            ("\0abstract", "NUL"),
        ] {
            let e = check_socket_path(Path::new(p)).unwrap_err();
            assert!(
                e.to_string().contains(frag),
                "{p:?} must be refused for `{frag}`, got {e}"
            );
        }
        let long = format!("/{}", "a".repeat(SUN_PATH_BYTES - 1));
        assert_eq!(long.len(), SUN_PATH_BYTES);
        assert!(
            check_socket_path(Path::new(&long))
                .unwrap_err()
                .to_string()
                .contains("sun_path")
        );
        let fits = format!("/{}", "a".repeat(SUN_PATH_BYTES - 2));
        assert_eq!(check_socket_path(Path::new(&fits)), Ok(()));
        // and connect refuses before any syscall
        assert!(matches!(
            unix_connect(Path::new("rel.sock")),
            Err(RawError::BadSocketPath { .. })
        ));
    }

    #[test]
    fn a_connect_with_no_listener_fails_at_once_with_its_errno() {
        let p = scratch("absent");
        let e = unix_connect(&p).unwrap_err();
        assert!(
            matches!(e, RawError::Syscall { errno: Some(n), .. } if n == libc::ENOENT),
            "{e:?}"
        );
    }

    /// ★ No "in progress" for AF_UNIX: once the listener's backlog is full a non-blocking
    /// connect returns EAGAIN, and that is a failed attempt (`is_would_block`).
    ///
    /// ⊘ CORRECTED 2026-10-03 (review of `v3-broker`): this used std's listener as bound, whose
    /// backlog is `somaxconn` (4096 here), so reaching EAGAIN took more than 4096 open client
    /// descriptors — under `ulimit -n 1024` the loop hit `EMFILE` first and failed on "the only
    /// refusal is EAGAIN" (measured). The listener is now re-armed with a backlog of 1, so the
    /// test needs a handful of descriptors whatever the host's limits are.
    #[test]
    fn a_full_backlog_is_eagain_and_never_in_progress() {
        let p = scratch("backlog");
        let l = UnixListener::bind(&p).expect("bind");
        // Linux re-applies `listen` on a listening AF_UNIX socket (`unix_listen` stores the new
        // `sk_max_ack_backlog`), so this narrows std's backlog of somaxconn to 1.
        // SAFETY: two integers by value; `l` is a live listening socket this test owns.
        let rc = unsafe { libc::listen(l.as_raw_fd(), 1) };
        assert_eq!(rc, 0, "re-listen with a backlog of 1");
        let mut held = Vec::new();
        let mut saw_eagain = false;
        for _ in 0..16 {
            match unix_connect(&p) {
                Ok(s) => held.push(s),
                Err(e) => {
                    assert!(
                        e.is_would_block(),
                        "the only refusal a live listener gives is EAGAIN, got {e:?}"
                    );
                    saw_eagain = true;
                    break;
                }
            }
        }
        assert!(
            saw_eagain,
            "a listener that never accepts must fill its backlog ({} connects)",
            held.len()
        );
        assert!(
            (1..=2).contains(&held.len()),
            "a backlog of 1 holds one or two pending connects (af_unix's `>` test), not {}",
            held.len()
        );
        drop(l);
        let _ = std::fs::remove_file(&p);
    }

    /// `effective_uid` is the effective uid the kernel reports for this process (the second
    /// field of `/proc/self/status`'s `Uid:` line), and the uid `SO_PEERCRED` names for a socket
    /// this process listens on.
    #[test]
    fn the_effective_uid_is_what_proc_and_so_peercred_say() {
        let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
        let euid: u32 = status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .and_then(|r| r.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .expect("an effective uid in /proc/self/status");
        assert_eq!(effective_uid(), euid);
        let p = scratch("euid");
        let l = UnixListener::bind(&p).expect("bind");
        let s = unix_connect(&p).expect("connect");
        assert_eq!(
            peer_credentials(s.as_fd()).expect("SO_PEERCRED").uid,
            effective_uid()
        );
        drop(l);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn peer_credentials_name_this_process() {
        let p = scratch("cred");
        let l = UnixListener::bind(&p).expect("bind");
        let s = unix_connect(&p).expect("connect");
        let c = peer_credentials(s.as_fd()).expect("SO_PEERCRED");
        assert_eq!(c.pid, std::process::id() as i32);
        let me = std::fs::metadata(&p)
            .map(|m| std::os::unix::fs::MetadataExt::uid(&m))
            .expect("stat the socket");
        assert_eq!(
            c.uid, me,
            "the socket file we bound is ours, as is the listener"
        );
        drop(l);
        let _ = std::fs::remove_file(&p);
    }

    /// One record with one descriptor crosses whole, and the descriptor is the same file.
    #[test]
    fn an_scm_rights_record_round_trips_on_a_socketpair() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let ram = crate::SharedRam::create(4096).expect("memfd");
        let sent = send_record(a.as_fd(), &[7u8; 40], Some(ram.as_backing_fd())).expect("send");
        assert_eq!(sent, 40);
        // the broker's side of the wire: a reader WITH a control buffer (test-only, below)
        let (body, fd) = recv_one_fd(&b);
        assert_eq!(body, vec![7u8; 40]);
        let got = std::fs::File::from(fd.expect("a descriptor crossed"));
        use std::os::unix::fs::MetadataExt as _;
        let theirs = got.metadata().expect("fstat").ino();
        let ours = std::fs::File::from(ram.dup_for_export().expect("dup"))
            .metadata()
            .expect("fstat")
            .ino();
        assert_eq!(theirs, ours, "the receiver holds the same memfd");
    }

    /// ★ The receive door accepts no descriptor: one sent to it is dropped by the kernel and
    /// reported, and the bytes still arrive.
    #[test]
    fn a_descriptor_sent_to_the_bounded_reader_is_dropped_and_reported() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let ram = crate::SharedRam::create(4096).expect("memfd");
        send_record(a.as_fd(), &[1u8; 24], Some(ram.as_backing_fd())).expect("send");
        let mut buf = [0u8; 64];
        let r = recv_bounded(b.as_fd(), &mut buf).expect("recv");
        assert_eq!(r.bytes, 24);
        assert!(r.control_dropped, "MSG_CTRUNC must be reported");
        // a plain record afterwards carries no flag
        (&a).write_all(&[2u8; 24]).expect("write");
        let r = recv_bounded(b.as_fd(), &mut buf).expect("recv");
        assert_eq!((r.bytes, r.control_dropped), (24, false));
        // nothing left: EAGAIN, never a block
        let e = recv_bounded(b.as_fd(), &mut buf).unwrap_err();
        assert!(e.is_would_block(), "{e:?}");
        drop(a);
        let r = recv_bounded(b.as_fd(), &mut buf).expect("eof");
        assert_eq!(r.bytes, 0, "a closed peer reads as end of stream");
        let mut rest = Vec::new();
        let _ = (&b).read_to_end(&mut rest);
    }

    #[test]
    fn a_full_socket_refuses_with_eagain_instead_of_blocking() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        let rec = [0u8; 40];
        let mut n = 0usize;
        loop {
            match send_record(a.as_fd(), &rec, None) {
                Ok(k) => n += k,
                Err(e) => {
                    assert!(e.is_would_block(), "{e:?}");
                    break;
                }
            }
            assert!(n < 64 << 20, "a socket buffer that never fills");
        }
        assert!(n > 0);
    }

    /// Test-only: receive one record and at most one descriptor (the broker's side of the
    /// wire). It lives in this file because it is a syscall.
    fn recv_one_fd(s: &UnixStream) -> (Vec<u8>, Option<OwnedFd>) {
        let mut body = [0u8; 64];
        let mut cmsg = [0u64; ONE_FD_CMSG_U64S];
        let mut iov = libc::iovec {
            iov_base: body.as_mut_ptr().cast::<libc::c_void>(),
            iov_len: body.len(),
        };
        let mut msg = zeroed_msghdr();
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg.as_mut_ptr().cast::<libc::c_void>();
        msg.msg_controllen = core::mem::size_of_val(&cmsg) as _;
        // SAFETY: test-only. `msg` points at `iov` (describing `body`) and at `cmsg`, both live
        // locals sized by their own `size_of`; `recvmsg` writes within them. The one descriptor
        // read out of the header (if any) is adopted immediately and owned by the caller.
        unsafe {
            let n = libc::recvmsg(s.as_raw_fd(), &raw mut msg, 0);
            assert!(n >= 0, "recvmsg");
            let hdr = libc::CMSG_FIRSTHDR(&raw const msg);
            let fd = if hdr.is_null() {
                None
            } else {
                let mut raw: libc::c_int = -1;
                core::ptr::copy_nonoverlapping(
                    libc::CMSG_DATA(hdr),
                    (&raw mut raw).cast::<u8>(),
                    core::mem::size_of::<libc::c_int>(),
                );
                Some(adopt_fd(raw, "test recvmsg").expect("a received descriptor"))
            };
            (body[..n as usize].to_vec(), fd)
        }
    }
}
