//! The isolate ⇄ VMM descriptor crossing: framing, provenance, and the two refusals.
//!
//! `isolate_vmm_fd_crossing.md`. [`kayfabe_linux_raw`] owns the syscalls — one `sendmsg`
//! with an `SCM_RIGHTS` control message, one `recvmsg` that adopts whatever came back,
//! and an `fstat` that says what it really is. This module owns everything above them:
//!
//! - **Framing.** The descriptors ride the frame's *first byte*, so a length word written
//!   separately from its body would strand them. [`write_frame_with_fds`] and
//!   [`read_frame_with_fds`] are the fd-carrying twins of [`crate::proto::write_frame`]
//!   and [`crate::proto::read_frame`], and they keep that invariant.
//! - **Provenance.** [`CrossedFd`] remembers *who handed it over*, which is the only way
//!   the cross-isolate refusal below can exist at all.
//!
//! ## ★★★ Why a descriptor is not just an `OwnedFd` here
//!
//! A descriptor is an **owning resource with a source**. Two properties have to travel
//! with it, and neither can be recovered later from the descriptor itself:
//!
//! 1. **What it is.** The message body carries the sender's *claim*; the kernel knows the
//!    truth. [`CrossedFd::adopt`] refuses the mismatch by name
//!    ([`RawError::DescriptorKindRefused`]) before the descriptor is usable, because the
//!    next thing the VMM does with a "GPU device" is `mmap` it and install the result as a
//!    guest memslot.
//! 2. **Whose it is.** Per-process isolates are the architecture and `#14` — two
//!    concurrent CUDA applications — is this rewrite's founding problem. A descriptor that
//!    came from isolate A must never be handed to isolate B. [`CrossedFd::lend_to`] is the
//!    only way to get a sendable borrow out of one, and it takes the target's identity so
//!    the check cannot be skipped by forgetting to call it.
//!
//! ⊘ The C has neither. `C: src/qemu/nvkvm_isolate.c:441-462` gates on the *message type*
//! that may legitimately carry a descriptor and closes the rest — which stops the
//! descriptor-table DoS its own R2-M1 audit found, and says nothing about what the
//! descriptor **is** or **whose** it is. Porting that gate alone would reproduce both gaps.
//! The C's one cross-isolate transfer (`ISOLATE_CMD_XISO_IMPORT`, its #110) is a
//! *deliberately brokered* dma-buf import for the graphics path, guarded by a comment
//! rather than a check — see §7 of the design note for why that is a separate, explicitly
//! argued exception and not a reason to relax the default.

use crate::proto::FRAME_MAX;
use kayfabe_isolate::IsolateId;
use kayfabe_linux_raw::{
    DescriptorKind, MAX_FDS_PER_FRAME, RawError, recv_with_fds, send_with_fds,
};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

/// Where a descriptor came from.
///
/// Not decoration: [`CrossedFd::lend_to`] reads it, and it is the whole of the
/// cross-isolate check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdOrigin {
    /// The VMM minted it — a `memfd` holding a ring or a slice of shareable guest RAM
    /// (`l1_os_shell.md` §4.4.1). The C's `ISOLATE_CMD_RECEIVE_FD` / `SETUP_RING`
    /// direction. It belongs to no isolate, so it may be handed to any of them.
    Vmm,
    /// An isolate opened it on the GPU's behalf and handed it back — the C's
    /// `ISOLATE_RESP_OPEN_DEVICE` direction, and the one this port lacked. It belongs to
    /// **that** isolate and may go back only there.
    Isolate(IsolateId),
}

impl FdOrigin {
    /// A stable `u64` name for a refusal message. `u64::MAX` is the VMM, which is not an
    /// isolate and cannot collide with one.
    fn as_u64(self) -> u64 {
        match self {
            FdOrigin::Vmm => u64::MAX,
            FdOrigin::Isolate(id) => (u64::from(id.proc()) << 32) | u64::from(id.gpu().0),
        }
    }
}

/// A descriptor that crossed the isolate boundary, carrying **what it is** and **whose it
/// is**.
///
/// Constructed only by [`CrossedFd::adopt`], which runs the kind check, so a `CrossedFd`
/// that exists is a descriptor whose type has been verified against the kernel — pinned by
/// `crates/kayfabe-isolate-host/tests/fd_crossing.rs` (three kind refusals, watched to fire
/// with the check removed).
#[derive(Debug)]
pub struct CrossedFd {
    fd: OwnedFd,
    origin: FdOrigin,
    kind: DescriptorKind,
}

impl CrossedFd {
    /// Adopt a received descriptor, **checking it is what the protocol promised**.
    ///
    /// ★ Takes the `OwnedFd` by value: on the refusal path it is dropped here, so a
    /// descriptor that fails validation is *closed*, never leaked. That is the whole
    /// reason this takes ownership rather than a borrow — a validator that borrows leaves
    /// the caller holding an open descriptor it has just been told to distrust.
    ///
    /// # Errors
    /// [`RawError::DescriptorKindRefused`] if the kernel disagrees with `promised`.
    pub fn adopt(
        fd: OwnedFd,
        origin: FdOrigin,
        promised: DescriptorKind,
    ) -> Result<Self, RawError> {
        kayfabe_linux_raw::require_kind(fd.as_fd(), promised)?;
        Ok(CrossedFd {
            fd,
            origin,
            kind: promised,
        })
    }

    /// Who handed this over.
    #[must_use]
    pub fn origin(&self) -> FdOrigin {
        self.origin
    }

    /// What the kernel says it is — established at [`CrossedFd::adopt`], never claimed by
    /// the peer.
    #[must_use]
    pub fn kind(&self) -> DescriptorKind {
        self.kind
    }

    /// ★★★ Borrow it for delivery **to a named isolate**, or refuse.
    ///
    /// The target's identity is a parameter rather than something the caller checks
    /// beforehand, because *"the caller checks first"* is exactly the property that
    /// decays. There is no other way to obtain a borrow for sending.
    ///
    /// - A [`FdOrigin::Vmm`] descriptor goes to any isolate: the VMM minted it and it
    ///   names no isolate's objects.
    /// - An [`FdOrigin::Isolate`] descriptor goes back **only** to the isolate it came
    ///   from. Anywhere else is [`RawError::ForeignDescriptor`] — a live handle onto
    ///   isolate A's GPU objects landing in isolate B's table is the `#14` breach, and it
    ///   is refused by name rather than prevented by an assumption about topology.
    ///
    /// # Errors
    /// [`RawError::ForeignDescriptor`], naming both isolates.
    pub fn lend_to(&self, target: IsolateId) -> Result<BorrowedFd<'_>, RawError> {
        match self.origin {
            FdOrigin::Vmm => Ok(self.fd.as_fd()),
            FdOrigin::Isolate(owner) if owner == target => Ok(self.fd.as_fd()),
            FdOrigin::Isolate(_) => Err(RawError::ForeignDescriptor {
                origin: self.origin.as_u64(),
                target: FdOrigin::Isolate(target).as_u64(),
            }),
        }
    }

    /// Borrow it for the VMM's **own** use — `mmap`, `ioctl`, memslot installation.
    ///
    /// Unrestricted by design: the VMM is the side that received it, and §5 of the design
    /// note states plainly what it may and may not do with one.
    #[must_use]
    pub fn as_local_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Give up ownership — for the one caller that must hold the descriptor beyond this
    /// type's lifetime. ⚠ The provenance is dropped with the wrapper, so this is the
    /// point past which the cross-isolate check can no longer be made.
    #[must_use]
    pub fn into_owned(self) -> OwnedFd {
        self.fd
    }
}

/// Everything that can go wrong carrying a frame that may hold descriptors.
///
/// Separate variants rather than one "bad frame", because the test doctrine asserts
/// **exact** variants: a truncation test that passes because the length was oversize has
/// tested nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdFrameError {
    /// The peer died between a frame's length and its body. Distinguished from a clean end
    /// of stream, which is [`read_frame_with_fds`] returning `Ok(false)`: one is a corpse,
    /// the other an orderly shutdown.
    Incomplete {
        /// Where in the frame it stopped.
        what: &'static str,
    },
    /// The peer declared a length beyond [`FRAME_MAX`]. Refused **without reading it** —
    /// a length the peer controls must not make us allocate.
    Oversize {
        /// What the peer asked for.
        declared: usize,
    },
    /// The OS refused, or the boundary did — including
    /// [`RawError::TooManyDescriptors`].
    Os(RawError),
}

impl From<RawError> for FdFrameError {
    fn from(e: RawError) -> Self {
        FdFrameError::Os(e)
    }
}

impl core::fmt::Display for FdFrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FdFrameError::Incomplete { what } => write!(f, "the peer died {what}"),
            FdFrameError::Oversize { declared } => {
                write!(
                    f,
                    "the peer declared a {declared}-byte frame, beyond FRAME_MAX"
                )
            }
            FdFrameError::Os(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FdFrameError {}

/// Write one length-prefixed frame with `fds` attached.
///
/// ★ The length word and the body go in **one** call, exactly as
/// [`crate::proto::write_frame`] does — and here it is load-bearing twice over. Ancillary
/// data attaches to the first byte the kernel accepts; a length written separately would
/// deliver the descriptors with a word the reader consumes before it knows what frame they
/// belong to.
///
/// # Errors
/// [`FdFrameError::Oversize`] if `body` exceeds [`FRAME_MAX`], or [`FdFrameError::Os`]
/// carrying the syscall's refusal.
pub fn write_frame_with_fds(
    sock: BorrowedFd<'_>,
    body: &[u8],
    fds: &[BorrowedFd<'_>],
) -> Result<(), FdFrameError> {
    if body.len() > FRAME_MAX {
        return Err(FdFrameError::Oversize {
            declared: body.len(),
        });
    }
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_le_bytes());
    framed.extend_from_slice(body);
    send_with_fds(sock, &framed, fds)?;
    Ok(())
}

/// Read one length-prefixed frame, collecting up to `max_fds` descriptors that arrive
/// with it.
///
/// Returns `Ok(false)` on a clean end of stream. Descriptors land in `fds` already owned,
/// so **every** error path from here closes them.
///
/// ★★ The descriptors arrive with the frame's **length word**, because that is the first
/// byte of the sender's single `sendmsg`. So the control buffer is supplied on that read,
/// not on the body read — get this backwards and the descriptors are silently dropped by
/// the kernel while the bytes arrive perfectly.
///
/// `max_fds` is the per-message allowance: pass `0` for the messages the protocol says
/// carry none, and a peer that attaches one anyway has it closed by the kernel and the
/// frame refused with [`RawError::TooManyDescriptors`].
///
/// # Errors
/// [`FdFrameError::Incomplete`], [`FdFrameError::Oversize`], or [`FdFrameError::Os`].
pub fn read_frame_with_fds(
    sock: BorrowedFd<'_>,
    buf: &mut Vec<u8>,
    fds: &mut Vec<OwnedFd>,
    max_fds: usize,
) -> Result<bool, FdFrameError> {
    // The length word — and, with it, whatever descriptors the sender attached.
    let mut len_bytes = [0u8; 4];
    let mut filled = 0;
    while filled < 4 {
        let n = recv_with_fds(sock, &mut len_bytes[filled..], fds, max_fds)?;
        if n == 0 {
            if filled == 0 {
                return Ok(false);
            }
            return Err(FdFrameError::Incomplete {
                what: "inside a frame's length",
            });
        }
        filled += n;
    }

    let len = u32::from_le_bytes(len_bytes) as usize;
    if len > FRAME_MAX {
        return Err(FdFrameError::Oversize { declared: len });
    }
    buf.clear();
    buf.resize(len, 0);

    // The body. A well-behaved peer attaches nothing here — but a peer is not obliged to
    // be well-behaved, so the same allowance is applied rather than none: passing `0`
    // would let a peer that split its send bypass the cap that the length-word read
    // enforced.
    let mut filled = 0;
    while filled < len {
        let n = recv_with_fds(sock, &mut buf[filled..], fds, max_fds)?;
        if n == 0 {
            return Err(FdFrameError::Incomplete {
                what: "between a frame's length and its body",
            });
        }
        filled += n;
    }
    if fds.len() > max_fds {
        return Err(FdFrameError::Os(RawError::TooManyDescriptors {
            limit: max_fds,
        }));
    }
    Ok(true)
}

/// The boundary's own bound, re-exported so a caller can state its allowance against it
/// without reaching past this module.
pub const MAX_FDS: usize = MAX_FDS_PER_FRAME;
