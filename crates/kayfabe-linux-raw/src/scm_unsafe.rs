//! Descriptor passing over a unix socket — `SCM_RIGHTS`, both directions.
//!
//! `isolate_vmm_fd_crossing.md`. This is the raw-adapter half of the crossing the C
//! artifact has had since its `ISOLATE_CMD_RECEIVE_FD` / `ISOLATE_RESP_OPEN_DEVICE`
//! (`C: src/common/nvkvm_isolate_proto.h:43,67`) and this port lacked entirely: a
//! descriptor opened on one side of the isolate boundary, used on the other.
//!
//! Three doors, and the third is the one that matters:
//!
//! - [`send_with_fds`] — one `sendmsg` carrying a byte body **and** descriptors.
//! - [`recv_with_fds`] — one `recvmsg`, returning whatever descriptors came with it,
//!   already owned so that every error path closes them.
//! - [`require_kind`] — ★ **the refusal**. A descriptor that arrived over a socket is
//!   peer-controlled input; this is where the protocol's promise about it is checked
//!   against what the kernel says it actually is.
//!
//! ## ★★★ Why the received descriptors are adopted BEFORE they are judged
//!
//! [`recv_with_fds`] turns every raw integer the kernel wrote into the control buffer
//! into an `OwnedFd` **first**, and only then applies the count bound. The order is
//! deliberate and it is the Rust-native form of a refusal the C had to write by hand:
//!
//! > *"A compromised stub could attach a fd to ANY other response type; if we don't
//! > consume it, it leaks into QEMU's fd table (eventual fd-exhaustion DoS of the VMM).
//! > Close any received fd on every non-OPEN_DEVICE response."*
//! > — `C: src/qemu/nvkvm_isolate.c:441-447` (audit finding R2-M1)
//!
//! The C closes them with an explicit loop that a later `case` can forget to run. Here
//! the descriptors are owned the instant they exist, so the leak is closed by `Drop` on
//! **every** path out of this function — including the ones that refuse. There is no
//! arm to forget.
//!
//! ## What this module does NOT decide
//!
//! It does not decide *which* message may carry a descriptor, or *what* the descriptor
//! is for. That is protocol policy and it lives with the protocol. This module answers
//! only "did descriptors arrive, are they too many, and is this one the kind it was
//! promised to be" — see `isolate_vmm_fd_crossing.md` §4 for the split.

use crate::error::{RawError, last_syscall_error};
use crate::host_fd_unsafe::adopt_fd;
use kayfabe_util::{leafwitness, lockwitness};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};

/// The most descriptors one frame may carry.
///
/// ★ A bound, not a capacity estimate. The control-message length is written by the
/// **peer**, and on the receive side that peer is the isolate — deliberately the less
/// trusted end of the boundary (`l1_os_shell.md` §11). Without a bound, a peer that
/// attaches descriptors to every frame exhausts the VMM's descriptor table, which is
/// the same DoS the C's R2-M1 finding names.
///
/// Four is chosen from the C's own traffic: no message in
/// `C: src/common/nvkvm_isolate_proto.h` carries more than one, and the headroom is for
/// a future multi-plane message rather than for a peer's ambition. The kernel enforces
/// it too — the control buffer is sized for exactly this many, so a peer that sends
/// more has the excess **closed by the kernel** before this code runs (`MSG_CTRUNC`),
/// and [`recv_with_fds`] then refuses the frame rather than silently using the prefix.
pub const MAX_FDS_PER_FRAME: usize = 4;

/// Control buffer, in `u64` units so its alignment satisfies `cmsghdr` (which is
/// `{ size_t, int, int }` — 8-byte aligned on every target this crate builds for).
/// Sized for [`MAX_FDS_PER_FRAME`] descriptors plus the header, generously rounded.
const CMSG_BUF_U64S: usize = 8;

/// What kind of object a descriptor refers to.
///
/// The variants are the file **type** bits of `st_mode`, which is the only property of a
/// received descriptor that can be checked cheaply and that cannot be forged by the
/// sender: the kernel reports what the object *is*, not what the peer said it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorKind {
    /// `S_IFCHR` — a character device. `/dev/nvidia0`, `/dev/nvidiactl`,
    /// `/dev/nvidia-uvm`: everything the isolate opens on the GPU's behalf.
    CharDevice,
    /// `S_IFREG` — a regular file, which is what a `memfd` reports as. The shareable
    /// backing of `l1_os_shell.md` §4.4.1 is one of these.
    RegularFile,
    /// Anything else a peer might attach: a socket, a pipe, a directory, an epoll set,
    /// another unix socket looping back to itself.
    Other {
        /// The raw `S_IFMT` bits, so a refusal says what actually arrived. Not a host
        /// address and not a path — a type code (§4.2.1 refusal 3 is satisfied).
        file_type: u32,
    },
}

impl DescriptorKind {
    /// Classify from the `S_IFMT` bits of an `st_mode`.
    fn from_mode(mode: u32) -> Self {
        match mode & libc::S_IFMT {
            libc::S_IFCHR => DescriptorKind::CharDevice,
            libc::S_IFREG => DescriptorKind::RegularFile,
            other => DescriptorKind::Other { file_type: other },
        }
    }
}

impl core::fmt::Display for DescriptorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DescriptorKind::CharDevice => write!(f, "a character device"),
            DescriptorKind::RegularFile => write!(f, "a regular file"),
            DescriptorKind::Other { file_type } => write!(
                f,
                "neither a character device nor a regular file (S_IFMT {file_type:#o})"
            ),
        }
    }
}

// =====================================================================================
// Sending
// =====================================================================================

/// Send `body` over `sock`, with `fds` attached to its first byte via `SCM_RIGHTS`.
///
/// Sends the **whole** body: a `SOCK_STREAM` socket may accept a prefix, and the
/// remainder is then sent by ordinary writes. ★ The ancillary data rides the **first**
/// `sendmsg` only, because that is where the kernel attaches it — the receiver sees the
/// descriptors together with the first byte of the body and never with a later one.
/// This is why the protocol layer above must put a frame's length word and its body in
/// one call: a descriptor that arrives attached to a length word the reader has already
/// consumed separately is a descriptor the reader cannot associate with anything.
///
/// Passing an empty `fds` is a plain send with no control message at all — not a send
/// with an empty `SCM_RIGHTS`, which is a different thing on the wire.
///
/// # Errors
/// [`RawError::Unsupported`] if `fds` exceeds [`MAX_FDS_PER_FRAME`] (a caller bug, not a
/// peer's — the peer cannot reach this side of the socket), [`RawError::ZeroLength`] for
/// an empty body, or [`RawError::Syscall`] carrying the `errno`.
///
/// # Panics
/// If a ranked core lock is held, or if called from a leaf context — both witnesses are
/// asserted at the door, as §4.5 requires of every syscall in this crate.
pub fn send_with_fds(
    sock: BorrowedFd<'_>,
    body: &[u8],
    fds: &[BorrowedFd<'_>],
) -> Result<(), RawError> {
    lockwitness::assert_lock_free("sending a descriptor across the isolate boundary");
    leafwitness::assert_leaf_free("sending a descriptor across the isolate boundary");

    if body.is_empty() {
        return Err(RawError::ZeroLength {
            what: "frame body accompanying a descriptor",
        });
    }
    if fds.len() > MAX_FDS_PER_FRAME {
        return Err(RawError::Unsupported {
            what: "that many descriptors in one frame",
            detail: "one frame carries at most MAX_FDS_PER_FRAME descriptors; \
                     split the message rather than raising the bound",
        });
    }

    let raw: [libc::c_int; MAX_FDS_PER_FRAME] = {
        let mut r = [-1; MAX_FDS_PER_FRAME];
        for (slot, fd) in r.iter_mut().zip(fds) {
            *slot = fd.as_raw_fd();
        }
        r
    };
    let payload_len = std::mem::size_of_val(&raw[..fds.len()]);

    let mut cmsg = [0u64; CMSG_BUF_U64S];
    let mut iov = libc::iovec {
        iov_base: body.as_ptr().cast::<libc::c_void>().cast_mut(),
        iov_len: body.len(),
    };
    let mut msg: libc::msghdr = unsafe_zeroed_msghdr();
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;

    if !fds.is_empty() {
        // SAFETY: `cmsg` is a live, 8-byte-aligned `[u64; CMSG_BUF_U64S]` in this frame
        // (u64 alignment is cmsghdr's alignment on every target this crate builds for),
        // and `msg_controllen` below is set from `cmsg`'s own size, so `CMSG_FIRSTHDR`
        // returns a pointer inside that array or null. `fds.len() <= MAX_FDS_PER_FRAME`
        // was checked above and `CMSG_BUF_U64S * 8` is sized for that many descriptors
        // plus a header, so `CMSG_SPACE(payload_len)` fits — asserted on the next line
        // rather than assumed. `raw[..fds.len()]` is initialised from `fds` above and is
        // live for this block; `copy_nonoverlapping` moves exactly `payload_len` bytes,
        // computed from that same slice, into the control payload the header just sized.
        unsafe {
            let space = libc::CMSG_SPACE(payload_len as libc::c_uint) as usize;
            assert!(
                space <= std::mem::size_of_val(&cmsg),
                "control buffer is sized for MAX_FDS_PER_FRAME descriptors"
            );
            msg.msg_control = cmsg.as_mut_ptr().cast::<libc::c_void>();
            msg.msg_controllen = space as _;

            let hdr = libc::CMSG_FIRSTHDR(&raw const msg);
            assert!(!hdr.is_null(), "a sized control buffer has a first header");
            (*hdr).cmsg_level = libc::SOL_SOCKET;
            (*hdr).cmsg_type = libc::SCM_RIGHTS;
            (*hdr).cmsg_len = libc::CMSG_LEN(payload_len as libc::c_uint) as _;
            std::ptr::copy_nonoverlapping(
                raw.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(hdr),
                payload_len,
            );
        }
    }

    // SAFETY: `msg` is fully initialised above — `msg_iov` points at `iov`, live in this
    // frame and describing `body`, which is live for the call; the control fields are
    // either both zero (no descriptors) or point at `cmsg`, live in this frame, with a
    // length that was asserted to fit it. `sendmsg` reads through these and writes
    // nothing. The return value is checked immediately below.
    let sent = unsafe { libc::sendmsg(sock.as_raw_fd(), &raw const msg, libc::MSG_NOSIGNAL) };
    if sent < 0 {
        return Err(last_syscall_error("sendmsg(SCM_RIGHTS)"));
    }
    let sent = sent as usize;
    if sent == body.len() {
        return Ok(());
    }

    // A stream socket took a prefix. The descriptors are already delivered — they went
    // with the first byte — so the tail is an ordinary write. Not a loop over `sendmsg`:
    // re-attaching the control message would send the descriptors a SECOND time.
    let mut done = sent;
    while done < body.len() {
        let tail = &body[done..];
        // SAFETY: `tail` is a non-empty subslice of `body`, live for this call, and the
        // length passed is `tail.len()` — computed here from the slice itself, never by a
        // caller. `send` reads that many bytes from it and writes nothing. The return
        // value is checked on the next line.
        let n = unsafe {
            libc::send(
                sock.as_raw_fd(),
                tail.as_ptr().cast::<libc::c_void>(),
                tail.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if n <= 0 {
            return Err(last_syscall_error("send(frame tail)"));
        }
        done += n as usize;
    }
    Ok(())
}

// =====================================================================================
// Receiving
// =====================================================================================

/// Receive up to `buf.len()` bytes from `sock`, appending any descriptors that arrive
/// with them to `fds` — at most `max_fds` of them.
///
/// Returns the number of bytes read; `Ok(0)` is a clean end of stream, which the
/// protocol layer reads as the peer having gone away rather than as an error.
///
/// ## ★★ `max_fds` is the cap, and the control buffer is sized from it
///
/// The caller states how many descriptors *this* message is allowed to carry, and the
/// control buffer is sized for exactly that many. Everything follows from that one
/// decision:
///
/// - A peer that attaches more gets `MSG_CTRUNC`, the kernel closes the excess itself,
///   and this function refuses the frame with [`RawError::TooManyDescriptors`].
/// - `max_fds == 0` means *"this message carries no descriptors"* and is the ordinary
///   case. It is the port of the C's R2-M1 sweep (`C: src/qemu/nvkvm_isolate.c:441-462`)
///   — but where the C closes stray descriptors in a loop a later `case` can forget to
///   run, here the kernel never hands them over at all.
/// - Because the cap is per-call rather than global, the refusal is **testable by
///   undersizing it**: send two, receive with `max_fds = 1`, watch it fire.
///
/// ★ Every descriptor the kernel *does* deliver is adopted into an `OwnedFd` **before**
/// any check runs, so a refusal below closes it rather than leaking it. Descriptors
/// arrive `CLOEXEC` — `MSG_CMSG_CLOEXEC` — because the VMM spawns isolates, and
/// `O_CLOEXEC` clears at the child's own `exec`, not at `fork`: a descriptor received
/// without it leaks into every child spawned between here and that `exec`.
///
/// # Errors
/// [`RawError::TooManyDescriptors`] if the peer exceeded `max_fds`,
/// [`RawError::Unsupported`] if `max_fds` exceeds [`MAX_FDS_PER_FRAME`],
/// [`RawError::ZeroLength`] for an empty buffer, or [`RawError::Syscall`] with the
/// `errno`.
///
/// # Panics
/// If a ranked core lock is held, or if called from a leaf context.
pub fn recv_with_fds(
    sock: BorrowedFd<'_>,
    buf: &mut [u8],
    fds: &mut Vec<OwnedFd>,
    max_fds: usize,
) -> Result<usize, RawError> {
    lockwitness::assert_lock_free("receiving a descriptor across the isolate boundary");
    leafwitness::assert_leaf_free("receiving a descriptor across the isolate boundary");

    if buf.is_empty() {
        return Err(RawError::ZeroLength {
            what: "receive buffer for a frame that may carry descriptors",
        });
    }
    if max_fds > MAX_FDS_PER_FRAME {
        return Err(RawError::Unsupported {
            what: "a descriptor allowance beyond the boundary's own bound",
            detail: "max_fds must not exceed MAX_FDS_PER_FRAME; raising the bound is a \
                     design change, not a call-site argument",
        });
    }

    let mut cmsg = [0u64; CMSG_BUF_U64S];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: buf.len(),
    };
    let mut msg: libc::msghdr = unsafe_zeroed_msghdr();
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.as_mut_ptr().cast::<libc::c_void>();
    // ★ Sized for `max_fds`, NOT for the array. This is the line the cap is made of: the
    // kernel fills the control buffer up to this length and sets `MSG_CTRUNC` when the
    // peer sent more, so undersizing it is exactly how a caller says "no more than this".
    // SAFETY: `CMSG_SPACE` is a pure size computation over a length this function
    // supplies; it dereferences nothing. `max_fds <= MAX_FDS_PER_FRAME` was checked
    // above, and `CMSG_BUF_U64S * 8` is sized for that many descriptors plus a header, so
    // the result cannot exceed `cmsg`'s own size — asserted on the following line rather
    // than assumed.
    let control_space = unsafe {
        libc::CMSG_SPACE((max_fds * std::mem::size_of::<libc::c_int>()) as libc::c_uint) as usize
    };
    assert!(
        control_space <= std::mem::size_of_val(&cmsg),
        "control buffer is sized for MAX_FDS_PER_FRAME descriptors"
    );
    msg.msg_controllen = control_space as _;

    // SAFETY: `msg` is fully initialised above — `msg_iov` points at `iov`, live in this
    // frame and describing `buf`, which is mutably borrowed for this call; `msg_control`
    // points at `cmsg`, live in this frame, with `msg_controllen` set from that array's
    // own size. `recvmsg` writes at most those lengths and nothing else. The return
    // value is checked immediately below.
    let got = unsafe {
        libc::recvmsg(
            sock.as_raw_fd(),
            &raw mut msg,
            libc::MSG_CMSG_CLOEXEC | libc::MSG_NOSIGNAL,
        )
    };
    if got < 0 {
        return Err(last_syscall_error("recvmsg(SCM_RIGHTS)"));
    }

    // Collect first, judge second. Anything adopted here is closed by `Drop` on every
    // path out of this function, including the refusals below.
    let mut received: Vec<OwnedFd> = Vec::new();
    // SAFETY: `msg` was just filled by a successful `recvmsg`, so `msg_control` /
    // `msg_controllen` describe the control data the KERNEL wrote into `cmsg`, which is
    // live in this frame for the whole walk; `CMSG_FIRSTHDR`/`CMSG_NXTHDR` are the
    // kernel's own iteration protocol over exactly that region and stop at null. For a
    // header the kernel itself wrote as `SCM_RIGHTS`, its payload is a run of `c_int`
    // descriptors whose count is derived from that header's own `cmsg_len` — never from
    // a peer-supplied number in the message body — and `read_unaligned` makes no
    // alignment claim about the payload. Each integer is handed straight to `adopt_fd`
    // on the expression that reads it, so each becomes an `OwnedFd` exactly once.
    unsafe {
        let mut hdr = libc::CMSG_FIRSTHDR(&raw const msg);
        while !hdr.is_null() {
            if (*hdr).cmsg_level == libc::SOL_SOCKET && (*hdr).cmsg_type == libc::SCM_RIGHTS {
                let payload = (*hdr).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let count = payload / std::mem::size_of::<libc::c_int>();
                let data = libc::CMSG_DATA(hdr).cast::<libc::c_int>();
                for i in 0..count {
                    let raw = data.add(i).read_unaligned();
                    received.push(adopt_fd(raw, "recvmsg(SCM_RIGHTS)")?);
                }
            }
            hdr = libc::CMSG_NXTHDR(&raw const msg, hdr);
        }
    }

    // ★★ The kernel truncates the control data when it does not fit and closes the excess
    // ITSELF, so `MSG_CTRUNC` is the only evidence it happened: a receiver that ignores
    // the flag believes it got everything while the sender believes it handed over more.
    // Refuse the frame whole; do NOT serve it from the prefix that did fit, which would
    // let a peer choose which of its descriptors we act on. `received` is dropped on this
    // path, so the ones that DID arrive are closed here.
    if msg.msg_flags & libc::MSG_CTRUNC != 0 || received.len() > max_fds {
        return Err(RawError::TooManyDescriptors { limit: max_fds });
    }

    fds.append(&mut received);
    Ok(got as usize)
}

// =====================================================================================
// ★★★ The refusal
// =====================================================================================

/// What kind of object `fd` actually refers to, according to the kernel.
///
/// # Errors
/// [`RawError::Syscall`] if `fstat` fails — which for a descriptor this process owns
/// means `EBADF`, i.e. a bug here rather than a peer's doing.
///
/// # Panics
/// If a ranked core lock is held, or if called from a leaf context.
pub fn descriptor_kind(fd: BorrowedFd<'_>) -> Result<DescriptorKind, RawError> {
    lockwitness::assert_lock_free("classifying a received descriptor");
    leafwitness::assert_leaf_free("classifying a received descriptor");

    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `st` is a live, correctly-aligned, writable `stat`-sized allocation in this
    // frame, which is the only thing `fstat` writes through; it never reads it. `fd` is a
    // `BorrowedFd`, so the descriptor is open for the duration of this call by the
    // borrow. The return value is checked on the next line, and `assume_init` runs only
    // on the success path — where the kernel has written the whole struct.
    let rc = unsafe { libc::fstat(fd.as_raw_fd(), st.as_mut_ptr()) };
    if rc != 0 {
        return Err(last_syscall_error("fstat(received descriptor)"));
    }
    // SAFETY: `fstat` returned 0 on the line above, which is its contract for having
    // written the complete `stat` structure.
    let st = unsafe { st.assume_init() };
    Ok(DescriptorKind::from_mode(st.st_mode))
}

/// ★★★ Refuse a received descriptor that is not the kind the protocol promised.
///
/// **A descriptor that arrived over a socket is peer-controlled input.** The message
/// body says what the sender claims it attached; this asks the kernel what it actually
/// is, and the two are independent. On the receive side of the VMM's socket the peer is
/// the **isolate** — deliberately the less trusted end (`l1_os_shell.md` §11) — so a
/// buggy or compromised isolate answering an "open me the GPU" request with, say, its
/// own end of a unix socket, a directory, or a descriptor onto an unrelated file is a
/// real vector and not a hypothetical one. Without this check the VMM would go on to
/// `mmap` it and install the result as a guest memslot.
///
/// ⊘ This is the check the C **does not** have: `C: src/qemu/nvkvm_isolate.c:441-462`
/// gates on the *message type* that may carry a descriptor and closes the rest, which
/// stops the descriptor-table DoS but says nothing about what the descriptor **is**.
/// Porting that gate without this one would reproduce the gap.
///
/// # Errors
/// [`RawError::DescriptorKindRefused`] naming both the promised and the actual kind, or
/// whatever [`descriptor_kind`] failed with.
///
/// # Panics
/// If a ranked core lock is held, or if called from a leaf context.
pub fn require_kind(fd: BorrowedFd<'_>, expected: DescriptorKind) -> Result<(), RawError> {
    let actual = descriptor_kind(fd)?;
    if actual == expected {
        return Ok(());
    }
    Err(RawError::DescriptorKindRefused { expected, actual })
}

/// A zeroed `msghdr`.
///
/// `libc::msghdr` has private padding on some targets, so it cannot be built with a
/// struct literal portably. Zero is the correct initial value for every field: null
/// control/name pointers with zero lengths is exactly "no ancillary data".
fn unsafe_zeroed_msghdr() -> libc::msghdr {
    // SAFETY: `msghdr` is a plain-old-data C struct of pointers, integers and padding —
    // it has no niche, no reference field and no invalid bit pattern — so all-zeroes is a
    // valid inhabitant. Every field is overwritten or deliberately left zero by the two
    // callers in this file before the struct reaches a syscall.
    unsafe { std::mem::zeroed() }
}
