//! The isolate wire protocol — one request, one reply, per worker channel.
//!
//! ## ★★ The C's shape, and the ONE place this deliberately diverges
//!
//! The C multiplexes **every** worker over a single socket and demultiplexes replies by
//! `txn_id` (`C: src/stub/nvkvm_stub.c:748` — one shared job queue, 16 workers, one fd 0).
//! That design needed a hardening pass whose comment is the argument against it: a stub
//! that echoed one `txn_id` twice would write into a **returned stack frame**, so the
//! reader must unlink the pending entry under the lock before receiving into the caller's
//! buffers (`C: src/qemu/nvkvm_isolate.c:485-505`, audit R2-H2).
//!
//! `l1_concurrency.md` §11 B6 rules the other way, in as many words: *"concurrency comes
//! from channel COUNT, never from multiplexing one channel"*. So each pool worker owns its
//! own socket, the channel is 1-deep, and **there is no demux, no pending list, and no
//! `txn_id` to confuse**. The entire class of bug the C's audit found is unrepresentable
//! here, and that is the reason for the divergence rather than a preference for sockets.
//!
//! The txn still travels ([`Envelope::txn`]) — but only so the *cancel* path can check the
//! C's refinement 4 (*"only signal the worker if it is still on that txn"*), never to route
//! a reply.
//!
//! ## ★ Two `RmError` variants have NO wire form, and that is load-bearing
//!
//! - [`kayfabe_isolate::RmError::ForeignHandle`] is produced by `Worker::execute`'s gate
//!   **before** a request is written. If the isolate could report it, a compromised child
//!   could claim our own invariant broke — and the gate's whole value is that it is *our*
//!   audit of *our* plan.
//! - [`kayfabe_isolate::RmError::Wedged`] is produced when the reply channel is **abandoned**
//!   (§7.5). A child that could send it would be reporting that it never answered, which is
//!   a contradiction: a wedged worker is by definition one that cannot write.
//!
//! `wire_errors_that_the_child_may_not_claim` asserts both.
//!
//! ## Framing
//!
//! `u32` little-endian length, then that many bytes. A length beyond [`FRAME_MAX`] is
//! refused **without reading it** — the child is inside the threat model's boundary 2, so a
//! length field it controls must not be able to make us allocate.

use kayfabe_isolate::RmError;
use std::io::{self, Read, Write};

/// The largest frame either side will send or accept.
///
/// Sized against the C's own `MAX_IOCTL_PAYLOAD` of 64 KiB
/// (`C: src/qemu/nvkvm_isolate.c:357`) with room for the control payloads RM controls
/// carry, and stated as a **refusal bound** rather than a buffer size: nothing here
/// pre-allocates it.
pub const FRAME_MAX: usize = 1 << 20;

/// What the parent asks the isolate to do. One variant per [`kayfabe_isolate::RmBackend`]
/// verb — the port is the protocol, so a verb added there fails to compile here until it is
/// carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// [`kayfabe_isolate::RmBackend::alloc`].
    Alloc {
        /// Parent handle, raw (the namespace is the connection).
        parent: u64,
        /// RM class id.
        class: u32,
        /// Already-encoded alloc parameters.
        params: Vec<u8>,
    },
    /// [`kayfabe_isolate::RmBackend::alloc_vaspace`].
    AllocVaSpace,
    /// [`kayfabe_isolate::RmBackend::alloc_sysmem`].
    AllocSysmem {
        /// Bytes requested.
        len: u64,
    },
    /// [`kayfabe_isolate::RmBackend::alloc_channel`].
    AllocChannel {
        /// Host VAS handle, raw.
        vas: u64,
        /// The channel's engine, as [`engine_code`].
        engine: u8,
    },
    /// [`kayfabe_isolate::RmBackend::alloc_engine_object`].
    AllocEngineObject {
        /// Host channel handle, raw.
        chan: u64,
        /// RM class id.
        class: u32,
        /// Already-encoded alloc parameters.
        params: Vec<u8>,
    },
    /// [`kayfabe_isolate::RmBackend::schedule`].
    Schedule {
        /// Host channel handle, raw.
        chan: u64,
    },
    /// [`kayfabe_isolate::RmBackend::free`].
    Free {
        /// Handle to free, raw.
        obj: u64,
    },
    /// [`kayfabe_isolate::RmBackend::control`].
    Control {
        /// Target object, raw.
        obj: u64,
        /// Command id.
        cmd: u32,
        /// In/out payload.
        payload: Vec<u8>,
    },
    /// [`kayfabe_isolate::RmBackend::map_gpu_va`].
    MapGpuVa {
        /// Host VAS handle, raw.
        vas: u64,
        /// Memory object, raw.
        memory: u64,
        /// Bytes to map.
        len: u64,
        /// ★ #102 — the host GPU VA to map AT (the guest VA, by address identity). Raw
        /// because the wire carries scalars; the child re-wraps it as a `GpuVa`.
        at: u64,
    },
    /// [`kayfabe_isolate::RmBackend::unmap_gpu_va`].
    UnmapGpuVa {
        /// Host VAS handle, raw.
        vas: u64,
        /// The GPU VA to unmap.
        gpu_va: u64,
    },
    /// [`kayfabe_isolate::RmBackend::ring_doorbell`].
    RingDoorbell {
        /// Host work-submit token.
        token: u64,
    },
    /// [`kayfabe_isolate::RmBackend::ce_copy`] — ONE sub-copy of a partitioned
    /// copy-engine request (`#102` stage C2). The engine is carried as a byte because
    /// the PLAN chose it and the child must not re-decide it: a wire that omitted it
    /// would leave the child to infer representability, which is address-plane
    /// knowledge it does not have.
    CeCopy {
        /// Host VAS handle, raw.
        vas: u64,
        /// Destination address.
        dst: u64,
        /// Source address, or the fill constant (see `src_is_const`).
        src: u64,
        /// Bytes.
        len: u64,
        /// `0` = `src` is an address, `1` = `src` is a constant pattern.
        src_is_const: u8,
        /// `0` = [`kayfabe_isolate::CeExecutor::HostCe`], `1` =
        /// [`kayfabe_isolate::CeExecutor::Ours`].
        by_ours: u8,
    },
    /// [`kayfabe_isolate::RmBackend::fb_read`] — read the fabricated aperture (`#102`
    /// stage C3). The reply carries the bytes **and** whether the aperture covered the
    /// range at all, because those are two different facts and the second one is what
    /// the walker turns into `MISS = FAULT`.
    FbRead {
        /// Guest-framebuffer-physical address to read.
        phys: u64,
        /// How many bytes. Bounded by the frame limit on the way back, not here — a
        /// request that asks for more than a reply can carry is refused by the child
        /// with a named error rather than truncated.
        len: u64,
    },
    /// [`kayfabe_isolate::RmBackend::export_surface`].
    ExportSurface {
        /// Memory object to export, raw.
        memory: u64,
    },
    /// ★★★ [`kayfabe_isolate::RmBackend::export_backing`] — *"perform the mapping and
    /// hand back memory"* (`isolate_vmm_fd_crossing.md` §12).
    ///
    /// ★ The **only** request whose reply may carry a descriptor. That is protocol
    /// policy, and it is expressed as the `max_fds` allowance the reader passes
    /// ([`crate::fdcross::read_frame_with_fds`]): every other reply is read with an
    /// allowance of **zero**, so a child that attaches one to an `Alloc` reply has it
    /// closed by the kernel and the frame refused — the port of the C's R2-M1 gate,
    /// enforced by the kernel rather than by a `case` a later edit can forget.
    ExportBacking {
        /// `0` = [`kayfabe_isolate::ExportSource::Fabricated`, `1` =
        /// [`kayfabe_isolate::ExportSource::HostDeviceMemory`]. A code we do not
        /// recognise is a refusal, never a default — a defaulted source would silently
        /// turn a request for the card's bytes into a fresh page of zeros.
        source: u8,
        /// The RM object, raw, when `source == 1`; ignored otherwise.
        memory: u64,
        /// Bytes wanted.
        len: u64,
        /// `0` = [`kayfabe_vmm::Prot::ReadWrite`], `1` = [`kayfabe_vmm::Prot::ReadOnly`].
        prot: u8,
    },
    /// ★★★★★ [`kayfabe_isolate::RmBackend::map_guest_ram`] — the VMM instructing the
    /// isolate to map a slice of **guest RAM** (`mode2_isolate_memory_boundary.md` §5).
    ///
    /// ⊘⊘ **It carries no descriptor, and that is the finding rather than a shortcut.**
    /// The obvious design — and the one the task framing assumed — is a first
    /// parent→child `SCM_RIGHTS` request carrying the guest-RAM `memfd`. It is not needed:
    /// §2 of that page says the fd is handed to the isolate at a **fixed, known number**,
    /// precisely so the seccomp filter installed afterwards can hardcode it, and
    /// [`kayfabe_linux_raw::FdGrant`] already delivers descriptors at fixed numbers at
    /// spawn. So this verb moves **scalars only**, which is also what makes the later
    /// `SECCOMP_RET_USER_NOTIF` match trivial: every argument of the `mmap` the child is
    /// about to issue is already in this frame.
    ///
    /// ★ The consequence worth writing down: the fd-IN gap on the request path is REAL
    /// (the child's reader is `read_frame`, with no control buffer at all) and is **not on
    /// this path**. Building it here would have been a whole mechanism serving a verb that
    /// does not want it.
    MapGuestRam {
        /// Byte offset into the guest-RAM block. VMM-derived — see
        /// [`kayfabe_isolate::GuestRamGrant`].
        offset: u64,
        /// Length in bytes. VMM-derived.
        len: u64,
        /// `0` = [`kayfabe_vmm::Prot::ReadWrite`], `1` = [`kayfabe_vmm::Prot::ReadOnly`].
        prot: u8,
    },
    /// [`kayfabe_isolate::RmBackend::unmap_guest_ram`].
    UnmapGuestRam {
        /// The isolate's raw name for the mapping, as a previous [`Request::MapGuestRam`]
        /// reply reported it.
        region: u64,
        /// Length in bytes.
        len: u64,
    },
    /// ★★★★★ [`kayfabe_isolate::RmBackend::describe_guest_ram`] — make an RM
    /// memory object out of guest pages this isolate already has mapped.
    ///
    /// ⊘ **It names a MAPPING, never a range.** There is no offset and no length in this
    /// frame beyond the mapping's own, because the range was authorized once, by the VMM,
    /// in the [`Request::MapGuestRam`] that produced `region`. A second pair of numbers
    /// here would be a range the *isolate* chose, which is
    /// `mode2_isolate_memory_boundary.md` §3's circularity re-entering through the door
    /// the previous verb closed.
    DescribeGuestRam {
        /// The isolate's raw name for the mapping.
        region: u64,
        /// Length in bytes, as the grant stated it. Carried for the reply-shaping check
        /// only — the child re-reads the mapping's own extent and does not trust this.
        len: u64,
    },
}

/// A request plus the checkout transaction it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// The [`kayfabe_isolate::Txn`] this request was issued under. Carried for the cancel
    /// path only (module docs).
    pub txn: u64,
    /// The request.
    pub request: Request,
}

/// What the isolate answers. Shapes, not verbs: the parent knows which verb it sent, and a
/// reply that re-declared it would be a second source of truth for the same fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// One freshly minted handle, raw.
    Handle(u64),
    /// A channel: its handle and its host work-submit token.
    HandleAndToken(u64, u64),
    /// Success with nothing to report.
    Unit,
    /// A control's payload, as the host wrote it back.
    Payload(Vec<u8>),
    /// A GPU virtual address.
    Va(u64),
    /// An exported surface.
    Surface(u64),
    /// ★★★ #102 stage C3 — the answer to a [`Request::FbRead`].
    ///
    /// Two fields, not one, and the second is not a length: `covered == false` means the
    /// isolate's fabricated aperture does not reach that address, which the walker turns
    /// into a loud `MISS = FAULT`. Encoding it as "zero bytes came back" would make it
    /// indistinguishable from a page of genuine zeros, and those are opposite facts.
    FbBytes {
        /// Whether a mapped extent covered the whole requested range.
        covered: bool,
        /// The bytes. Empty when `covered` is false.
        bytes: Vec<u8>,
    },
    /// ★★★ The answer to a [`Request::ExportBacking`] — the geometry of a backing whose
    /// **descriptor rides this frame's ancillary data**.
    ///
    /// ⊘ It carries no token. The child's index into its own export table is a child
    /// value, and a parent that adopted it would be letting the peer name a slot in the
    /// parent's registry — the same "the isolate is supplied by the caller, never by the
    /// wire" rule [`WireError::into_rm_error`] already applies to a `BadHandle`. The
    /// parent mints its own token when it adopts the descriptor.
    Backing {
        /// Byte offset into the backing at which the range begins.
        offset: u64,
        /// How many bytes are there.
        len: u64,
        /// What the isolate actually granted: `0` = read-write, `1` = read-only.
        prot: u8,
    },
    /// The verb failed.
    Failed(WireError),
}

/// The subset of [`RmError`] an isolate may report. See the module docs for the two
/// variants that are deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// [`RmError::InsufficientPermissions`].
    InsufficientPermissions,
    /// [`RmError::BadHandle`], carrying the raw value the host refused.
    BadHandle(u64),
    /// [`RmError::NoMemory`].
    NoMemory,
    /// [`RmError::Interrupted`] — the host call returned `EINTR`.
    Interrupted,
    /// [`RmError::Other`], carrying the host's opaque status.
    Other(u32),
    /// ★★★ [`RmError::NotExportableAsMemory`] — the named boundary of decision (b).
    ///
    /// A wire form of its own rather than an `Other(status)`, because the whole value of
    /// naming the boundary is lost if it arrives as an opaque number: a caller must be
    /// able to tell *"the bytes are on the card and cannot cross as memory"* from
    /// *"the host refused"*, and a status code makes those the same fact.
    NotExportableAsMemory(u64),
    /// ★★★ [`RmError::GuestRamUnavailable`] — the VM was not launched with a shared,
    /// fd-backed memory block, so this isolate has no guest RAM to map.
    ///
    /// Named on the wire for the same reason as the variant above: it is a **deployment**
    /// fact the isolate can observe and the VMM cannot, and arriving as an opaque status
    /// would make it indistinguishable from "the host refused" — which is the difference
    /// between *reconfigure the VM* and *file a bug*.
    GuestRamUnavailable,
}

impl WireError {
    /// Lift to the port's error type. **The isolate is supplied by the caller**, never by
    /// the wire: a `BadHandle` is stamped with the namespace we asked on, so a child cannot
    /// name another isolate's namespace even by lying.
    #[must_use]
    pub fn into_rm_error(self, isolate: kayfabe_isolate::IsolateId) -> RmError {
        match self {
            WireError::InsufficientPermissions => RmError::InsufficientPermissions,
            WireError::BadHandle(raw) => {
                RmError::BadHandle(kayfabe_isolate::HostHandle::new(isolate, raw))
            }
            WireError::NoMemory => RmError::NoMemory,
            WireError::Interrupted => RmError::Interrupted,
            WireError::Other(status) => RmError::Other(status),
            WireError::NotExportableAsMemory(raw) => RmError::NotExportableAsMemory {
                memory: kayfabe_isolate::HostHandle::new(isolate, raw),
            },
            WireError::GuestRamUnavailable => RmError::GuestRamUnavailable,
        }
    }
}

/// The wire code for [`kayfabe_isolate::ExportSource::Fabricated`].
pub const EXPORT_SOURCE_FABRICATED: u8 = 0;
/// The wire code for [`kayfabe_isolate::ExportSource::HostDeviceMemory`].
pub const EXPORT_SOURCE_HOST_DEVICE: u8 = 1;

/// The wire code for [`kayfabe_vmm::Prot::ReadWrite`].
pub const PROT_READ_WRITE: u8 = 0;
/// The wire code for [`kayfabe_vmm::Prot::ReadOnly`].
pub const PROT_READ_ONLY: u8 = 1;

/// [`kayfabe_vmm::Prot`]'s wire code. A dedicated mapping rather than a discriminant
/// cast, for [`engine_code`]'s reason: a reordered enum must not silently re-route a
/// protection, and the direction that fails open here is the dangerous one — a
/// read-only run installed read-write.
#[must_use]
pub fn prot_code(prot: kayfabe_vmm::Prot) -> u8 {
    match prot {
        kayfabe_vmm::Prot::ReadWrite => PROT_READ_WRITE,
        kayfabe_vmm::Prot::ReadOnly => PROT_READ_ONLY,
    }
}

/// The inverse of [`prot_code`]. `None` for an unknown code — **never** a default, and
/// specifically never a default of read-write.
#[must_use]
pub fn prot_from_code(code: u8) -> Option<kayfabe_vmm::Prot> {
    match code {
        PROT_READ_WRITE => Some(kayfabe_vmm::Prot::ReadWrite),
        PROT_READ_ONLY => Some(kayfabe_vmm::Prot::ReadOnly),
        _ => None,
    }
}

/// The engine tag's wire code. A dedicated mapping rather than `as u8` on the enum: the
/// core's [`kayfabe_arch::ids::EngineKind`] is free to gain, reorder or rename variants,
/// and a discriminant cast would silently re-route a channel to a different runlist — the
/// exact shape of the C's `engineType=0` bug (`dma_copy_class_alloc_params`, seam audit
/// GR-1).
#[must_use]
pub fn engine_code(engine: kayfabe_arch::ids::EngineKind) -> u8 {
    use kayfabe_arch::ids::EngineKind as E;
    match engine {
        E::GrCompute => 1,
        E::GrGraphics => 2,
        E::Ce => 3,
        E::NvEnc => 4,
        E::NvDec => 5,
        E::Other => 6,
    }
}

/// The inverse of [`engine_code`]. `None` for an unknown code — a wire value we do not
/// recognise is a refusal, never a default: defaulting to `Other` would route the channel
/// somewhere plausible and wrong.
#[must_use]
pub fn engine_from_code(code: u8) -> Option<kayfabe_arch::ids::EngineKind> {
    use kayfabe_arch::ids::EngineKind as E;
    Some(match code {
        1 => E::GrCompute,
        2 => E::GrGraphics,
        3 => E::Ce,
        4 => E::NvEnc,
        5 => E::NvDec,
        6 => E::Other,
        _ => return None,
    })
}

// =====================================================================================
// Encoding — hand-rolled, because the alternative is a serialization dependency in the
// one process boundary that faces a compromised peer.
// =====================================================================================

/// A malformed frame. Every variant names what was wrong, because the peer is inside the
/// threat model and "decode failed" is not a diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    /// The frame ended in the middle of a field.
    Truncated {
        /// What was being read.
        what: &'static str,
    },
    /// A tag byte that names no variant.
    UnknownTag {
        /// Which enum.
        what: &'static str,
        /// The byte.
        tag: u8,
    },
    /// A length field larger than [`FRAME_MAX`], or a byte-string longer than its frame.
    Oversize {
        /// What was being read.
        what: &'static str,
        /// The declared length.
        len: u64,
    },
    /// Bytes remained after a complete value was decoded — a peer appending to a frame is
    /// a peer we do not understand, and the tolerant reading is how a parser differential
    /// starts.
    TrailingBytes {
        /// How many.
        len: usize,
    },
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Truncated { what } => write!(f, "frame ended inside {what}"),
            ProtoError::UnknownTag { what, tag } => write!(f, "unknown {what} tag {tag}"),
            ProtoError::Oversize { what, len } => write!(f, "{what} declares {len} bytes"),
            ProtoError::TrailingBytes { len } => write!(f, "{len} bytes after a complete value"),
        }
    }
}

impl std::error::Error for ProtoError {}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, at: 0 }
    }
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], ProtoError> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(ProtoError::Truncated { what })?;
        let s = self
            .bytes
            .get(self.at..end)
            .ok_or(ProtoError::Truncated { what })?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self, what: &'static str) -> Result<u8, ProtoError> {
        Ok(self.take(1, what)?[0])
    }
    fn u32(&mut self, what: &'static str) -> Result<u32, ProtoError> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self, what: &'static str) -> Result<u64, ProtoError> {
        let b = self.take(8, what)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn blob(&mut self, what: &'static str) -> Result<Vec<u8>, ProtoError> {
        let len = self.u32(what)? as usize;
        if len > FRAME_MAX {
            return Err(ProtoError::Oversize {
                what,
                len: len as u64,
            });
        }
        Ok(self.take(len, what)?.to_vec())
    }
    fn finish(self) -> Result<(), ProtoError> {
        let left = self.bytes.len() - self.at;
        if left == 0 {
            Ok(())
        } else {
            Err(ProtoError::TrailingBytes { len: left })
        }
    }
}

fn put_blob(out: &mut Vec<u8>, blob: &[u8]) {
    out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
    out.extend_from_slice(blob);
}

impl Envelope {
    /// Encode this envelope's body (without the frame length).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        out.extend_from_slice(&self.txn.to_le_bytes());
        match &self.request {
            Request::Alloc {
                parent,
                class,
                params,
            } => {
                out.push(1);
                out.extend_from_slice(&parent.to_le_bytes());
                out.extend_from_slice(&class.to_le_bytes());
                put_blob(&mut out, params);
            }
            Request::AllocVaSpace => out.push(2),
            Request::AllocSysmem { len } => {
                out.push(3);
                out.extend_from_slice(&len.to_le_bytes());
            }
            Request::AllocChannel { vas, engine } => {
                out.push(4);
                out.extend_from_slice(&vas.to_le_bytes());
                out.push(*engine);
            }
            Request::AllocEngineObject {
                chan,
                class,
                params,
            } => {
                out.push(5);
                out.extend_from_slice(&chan.to_le_bytes());
                out.extend_from_slice(&class.to_le_bytes());
                put_blob(&mut out, params);
            }
            Request::Schedule { chan } => {
                out.push(6);
                out.extend_from_slice(&chan.to_le_bytes());
            }
            Request::Free { obj } => {
                out.push(7);
                out.extend_from_slice(&obj.to_le_bytes());
            }
            Request::Control { obj, cmd, payload } => {
                out.push(8);
                out.extend_from_slice(&obj.to_le_bytes());
                out.extend_from_slice(&cmd.to_le_bytes());
                put_blob(&mut out, payload);
            }
            Request::MapGpuVa {
                vas,
                memory,
                len,
                at,
            } => {
                out.push(9);
                out.extend_from_slice(&vas.to_le_bytes());
                out.extend_from_slice(&memory.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(&at.to_le_bytes());
            }
            Request::UnmapGpuVa { vas, gpu_va } => {
                out.push(10);
                out.extend_from_slice(&vas.to_le_bytes());
                out.extend_from_slice(&gpu_va.to_le_bytes());
            }
            Request::RingDoorbell { token } => {
                out.push(11);
                out.extend_from_slice(&token.to_le_bytes());
            }
            Request::ExportSurface { memory } => {
                out.push(12);
                out.extend_from_slice(&memory.to_le_bytes());
            }
            Request::CeCopy {
                vas,
                dst,
                src,
                len,
                src_is_const,
                by_ours,
            } => {
                out.push(13);
                out.extend_from_slice(&vas.to_le_bytes());
                out.extend_from_slice(&dst.to_le_bytes());
                out.extend_from_slice(&src.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
                out.push(*src_is_const);
                out.push(*by_ours);
            }
            Request::FbRead { phys, len } => {
                out.push(14);
                out.extend_from_slice(&phys.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
            Request::ExportBacking {
                source,
                memory,
                len,
                prot,
            } => {
                out.push(15);
                out.push(*source);
                out.extend_from_slice(&memory.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
                out.push(*prot);
            }
            Request::MapGuestRam { offset, len, prot } => {
                out.push(16);
                out.extend_from_slice(&offset.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
                out.push(*prot);
            }
            Request::UnmapGuestRam { region, len } => {
                out.push(17);
                out.extend_from_slice(&region.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
            Request::DescribeGuestRam { region, len } => {
                out.push(18);
                out.extend_from_slice(&region.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
        }
        out
    }

    /// Decode an envelope body.
    ///
    /// # Errors
    /// [`ProtoError`] — every malformation is a named variant.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtoError> {
        let mut c = Cursor::new(bytes);
        let txn = c.u64("envelope txn")?;
        let request = match c.u8("request tag")? {
            1 => Request::Alloc {
                parent: c.u64("alloc parent")?,
                class: c.u32("alloc class")?,
                params: c.blob("alloc params")?,
            },
            2 => Request::AllocVaSpace,
            3 => Request::AllocSysmem {
                len: c.u64("sysmem len")?,
            },
            4 => Request::AllocChannel {
                vas: c.u64("channel vas")?,
                engine: c.u8("channel engine")?,
            },
            5 => Request::AllocEngineObject {
                chan: c.u64("engine chan")?,
                class: c.u32("engine class")?,
                params: c.blob("engine params")?,
            },
            6 => Request::Schedule {
                chan: c.u64("schedule chan")?,
            },
            7 => Request::Free {
                obj: c.u64("free obj")?,
            },
            8 => Request::Control {
                obj: c.u64("control obj")?,
                cmd: c.u32("control cmd")?,
                payload: c.blob("control payload")?,
            },
            9 => Request::MapGpuVa {
                vas: c.u64("map vas")?,
                memory: c.u64("map memory")?,
                len: c.u64("map len")?,
                at: c.u64("map at")?,
            },
            10 => Request::UnmapGpuVa {
                vas: c.u64("unmap vas")?,
                gpu_va: c.u64("unmap va")?,
            },
            11 => Request::RingDoorbell {
                token: c.u64("doorbell token")?,
            },
            12 => Request::ExportSurface {
                memory: c.u64("export memory")?,
            },
            13 => Request::CeCopy {
                vas: c.u64("ce vas")?,
                dst: c.u64("ce dst")?,
                src: c.u64("ce src")?,
                len: c.u64("ce len")?,
                src_is_const: c.u8("ce src kind")?,
                by_ours: c.u8("ce engine")?,
            },
            14 => Request::FbRead {
                phys: c.u64("fb phys")?,
                len: c.u64("fb len")?,
            },
            15 => Request::ExportBacking {
                source: c.u8("export source")?,
                memory: c.u64("export memory")?,
                len: c.u64("export len")?,
                prot: c.u8("export prot")?,
            },
            16 => Request::MapGuestRam {
                offset: c.u64("guest ram offset")?,
                len: c.u64("guest ram len")?,
                prot: c.u8("guest ram prot")?,
            },
            17 => Request::UnmapGuestRam {
                region: c.u64("guest ram region")?,
                len: c.u64("guest ram len")?,
            },
            18 => Request::DescribeGuestRam {
                region: c.u64("guest ram region")?,
                len: c.u64("guest ram len")?,
            },
            tag => {
                return Err(ProtoError::UnknownTag {
                    what: "request",
                    tag,
                });
            }
        };
        c.finish()?;
        Ok(Envelope { txn, request })
    }
}

impl Reply {
    /// Encode this reply's body (without the frame length).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        match self {
            Reply::Handle(h) => {
                out.push(1);
                out.extend_from_slice(&h.to_le_bytes());
            }
            Reply::HandleAndToken(h, t) => {
                out.push(2);
                out.extend_from_slice(&h.to_le_bytes());
                out.extend_from_slice(&t.to_le_bytes());
            }
            Reply::Unit => out.push(3),
            Reply::Payload(p) => {
                out.push(4);
                put_blob(&mut out, p);
            }
            Reply::Va(va) => {
                out.push(5);
                out.extend_from_slice(&va.to_le_bytes());
            }
            Reply::Surface(s) => {
                out.push(6);
                out.extend_from_slice(&s.to_le_bytes());
            }
            Reply::FbBytes { covered, bytes } => {
                out.push(8);
                out.push(u8::from(*covered));
                put_blob(&mut out, bytes);
            }
            Reply::Backing { offset, len, prot } => {
                out.push(9);
                out.extend_from_slice(&offset.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
                out.push(*prot);
            }
            Reply::Failed(e) => {
                out.push(7);
                match e {
                    WireError::InsufficientPermissions => out.push(1),
                    WireError::BadHandle(raw) => {
                        out.push(2);
                        out.extend_from_slice(&raw.to_le_bytes());
                    }
                    WireError::NoMemory => out.push(3),
                    WireError::Interrupted => out.push(4),
                    WireError::Other(status) => {
                        out.push(5);
                        out.extend_from_slice(&status.to_le_bytes());
                    }
                    WireError::NotExportableAsMemory(raw) => {
                        out.push(6);
                        out.extend_from_slice(&raw.to_le_bytes());
                    }
                    WireError::GuestRamUnavailable => out.push(7),
                }
            }
        }
        out
    }

    /// Decode a reply body.
    ///
    /// # Errors
    /// [`ProtoError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtoError> {
        let mut c = Cursor::new(bytes);
        let reply = match c.u8("reply tag")? {
            1 => Reply::Handle(c.u64("handle")?),
            2 => Reply::HandleAndToken(c.u64("handle")?, c.u64("token")?),
            3 => Reply::Unit,
            4 => Reply::Payload(c.blob("payload")?),
            5 => Reply::Va(c.u64("va")?),
            6 => Reply::Surface(c.u64("surface")?),
            7 => Reply::Failed(match c.u8("error tag")? {
                1 => WireError::InsufficientPermissions,
                2 => WireError::BadHandle(c.u64("bad handle")?),
                3 => WireError::NoMemory,
                4 => WireError::Interrupted,
                5 => WireError::Other(c.u32("status")?),
                6 => WireError::NotExportableAsMemory(c.u64("unexportable memory")?),
                7 => WireError::GuestRamUnavailable,
                tag => {
                    return Err(ProtoError::UnknownTag {
                        what: "wire error",
                        tag,
                    });
                }
            }),
            8 => Reply::FbBytes {
                // ★ Any non-zero byte is `true`. A peer that wrote 2 said "covered", and
                // refusing the frame over a boolean's spelling would turn a harmless
                // encoding difference into a dead isolate.
                covered: c.u8("fb covered")? != 0,
                bytes: c.blob("fb bytes")?,
            },
            9 => Reply::Backing {
                offset: c.u64("backing offset")?,
                len: c.u64("backing len")?,
                prot: c.u8("backing prot")?,
            },
            tag => return Err(ProtoError::UnknownTag { what: "reply", tag }),
        };
        c.finish()?;
        Ok(reply)
    }
}

// =====================================================================================
// Framing
// =====================================================================================

/// Write one length-prefixed frame.
///
/// # Errors
/// [`io::Error`] from the underlying channel, or [`io::ErrorKind::InvalidInput`] if `body`
/// exceeds [`FRAME_MAX`].
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > FRAME_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame exceeds FRAME_MAX",
        ));
    }
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_le_bytes());
    framed.extend_from_slice(body);
    // ★ ONE write, not two. A length written separately from its body is a channel that a
    // signal-interrupted partial write desynchronises permanently — the desync hazard §7.2
    // forbids, reached by the cancellation path rather than by a peer.
    w.write_all(&framed)
}

/// Read one length-prefixed frame into `buf`.
///
/// Returns `Ok(false)` on a clean end of stream, which is how §7.5's **abandon** surfaces:
/// the requester's socket is shut down by the watchdog, the blocked read returns zero, and
/// the caller turns that into [`RmError::Wedged`]. It is a state, not an error.
///
/// # Errors
/// [`io::Error`], including [`io::ErrorKind::Interrupted`] — which the caller must classify
/// rather than retry, because on the isolate's side it means a cancel landed.
pub fn read_frame(r: &mut impl Read, buf: &mut Vec<u8>) -> io::Result<bool> {
    let mut len = [0u8; 4];
    match read_exact_or_eof(r, &mut len)? {
        0 => return Ok(false),
        4 => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "peer died inside a frame's length",
            ));
        }
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > FRAME_MAX {
        // ★ Refused WITHOUT reading it. The peer is inside boundary 2; a length it controls
        // must not be able to make us allocate, and must not be able to make us consume an
        // unbounded number of bytes looking for the end either.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "peer declared a frame beyond FRAME_MAX",
        ));
    }
    buf.clear();
    buf.resize(len, 0);
    if read_exact_or_eof(r, buf)? != len {
        // A length arrived and its body did not: the peer died mid-frame. Distinguished
        // from a clean EOF because the two mean different things — one is an orderly
        // shutdown, the other is a corpse.
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "peer died between a frame's length and its body",
        ));
    }
    Ok(true)
}

/// Fill `buf`, reporting **how many bytes arrived** rather than treating a short read as
/// an error. The caller needs the distinction: zero bytes before a frame is an orderly
/// shutdown (§7.5's abandon), while a short read *inside* one is a corpse.
///
/// `Interrupted` is propagated, never retried. Retrying is the ordinary `std` behaviour
/// and it is wrong here — on the isolate's side an interrupted read is a **cancel that
/// landed**, and swallowing it reproduces the `SA_RESTART` failure one layer up.
fn read_exact_or_eof(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            // ★★ THE INTERRUPT RULE, and it is position-dependent — the C learned this one
            // too (`C: src/stub/nvkvm_stub.c:564-588`: *"a stray SIGUSR1 can hit a worker
            // mid-send; retry rather than corrupt the response framing"*).
            //
            // At a frame BOUNDARY (`filled == 0`) an interrupt is a **cancel that landed**
            // and must be reported, because that is the whole §7.2 mechanism.
            //
            // MID-FRAME it is a stray signal that must be RETRIED, because abandoning half
            // a frame desynchronises the channel permanently — the next reader takes the
            // remainder of this frame as the start of the next one, which is §7.2's
            // "silent cross-transaction corruption" reached from the signal path instead
            // of from the abandon path.
            Err(e) if e.kind() == io::ErrorKind::Interrupted && filled > 0 => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kayfabe_arch::ids::EngineKind;

    fn every_request() -> Vec<Request> {
        vec![
            Request::Alloc {
                parent: 0xC1D0_0001,
                class: 0x90f1,
                params: vec![1, 2, 3],
            },
            Request::AllocVaSpace,
            Request::AllocSysmem { len: 0x4000 },
            Request::AllocChannel {
                vas: 7,
                engine: engine_code(EngineKind::Ce),
            },
            Request::AllocEngineObject {
                chan: 9,
                class: 0xc7c0,
                params: Vec::new(),
            },
            Request::Schedule { chan: 9 },
            Request::Free { obj: 11 },
            Request::Control {
                obj: 13,
                cmd: 0x801813,
                payload: vec![0xAA; 64],
            },
            Request::MapGpuVa {
                vas: 7,
                memory: 11,
                len: 0x1000,
                at: 0x2_0020_0000,
            },
            Request::UnmapGpuVa {
                vas: 7,
                gpu_va: 0x200_0000,
            },
            Request::RingDoorbell { token: 0xDEAD },
            Request::ExportSurface { memory: 11 },
            // Both engine arms and both source kinds, so a wire that dropped either
            // discriminant round-trips WRONG rather than round-tripping short.
            Request::CeCopy {
                vas: 7,
                dst: 0x2_0000_0000,
                src: 0x3_0000_0000,
                len: 0x1000,
                src_is_const: 0,
                by_ours: 0,
            },
            Request::CeCopy {
                vas: 7,
                dst: 0x2_0000_0000,
                src: 0xdead_beef,
                len: 4,
                src_is_const: 1,
                by_ours: 1,
            },
            Request::FbRead {
                phys: 0x1000_4000,
                len: 4096,
            },
            // ★ BOTH sources, because the two arms of the verb have opposite outcomes and
            // a sample carrying only the one that succeeds would prove the round trip
            // about half the vocabulary.
            Request::ExportBacking {
                source: EXPORT_SOURCE_FABRICATED,
                memory: 0,
                len: 0x20_0000,
                prot: PROT_READ_WRITE,
            },
            Request::ExportBacking {
                source: EXPORT_SOURCE_HOST_DEVICE,
                memory: 0x11,
                len: 0x1000,
                prot: PROT_READ_ONLY,
            },
            // ★ BOTH protections, for the reason `prot_code` exists at all: the direction
            // that fails open here is the dangerous one, and a sample carrying only
            // read-write would prove the round trip about the harmless half.
            Request::MapGuestRam {
                offset: 0x237f_e000,
                len: 0x1_0000,
                prot: PROT_READ_WRITE,
            },
            Request::MapGuestRam {
                offset: 0,
                len: 0x1000,
                prot: PROT_READ_ONLY,
            },
            Request::UnmapGuestRam {
                region: 0x4000_0000_0000_0001,
                len: 0x1_0000,
            },
            Request::DescribeGuestRam {
                region: 0x4000_0000_0000_0001,
                len: 0x1_0000,
            },
        ]
    }

    /// ★★ **The name of a request's variant, as an exhaustive match.**
    ///
    /// This exists so that `every_request()` cannot silently shrink. A list-quantified
    /// gate is only as strong as its list, and the list is hand-written: adding a variant
    /// without adding a sample would leave the round-trip proved about everything except
    /// the new thing. Adding a variant breaks *this* match at compile time, and the
    /// coverage test below then fails until the sample exists.
    fn request_tag(r: &Request) -> &'static str {
        match r {
            Request::Alloc { .. } => "Alloc",
            Request::AllocVaSpace => "AllocVaSpace",
            Request::AllocSysmem { .. } => "AllocSysmem",
            Request::AllocChannel { .. } => "AllocChannel",
            Request::AllocEngineObject { .. } => "AllocEngineObject",
            Request::Schedule { .. } => "Schedule",
            Request::Free { .. } => "Free",
            Request::Control { .. } => "Control",
            Request::MapGpuVa { .. } => "MapGpuVa",
            Request::UnmapGpuVa { .. } => "UnmapGpuVa",
            Request::RingDoorbell { .. } => "RingDoorbell",
            Request::CeCopy { .. } => "CeCopy",
            Request::FbRead { .. } => "FbRead",
            Request::ExportSurface { .. } => "ExportSurface",
            Request::ExportBacking { .. } => "ExportBacking",
            Request::MapGuestRam { .. } => "MapGuestRam",
            Request::UnmapGuestRam { .. } => "UnmapGuestRam",
            Request::DescribeGuestRam { .. } => "DescribeGuestRam",
        }
    }

    #[test]
    fn the_round_trip_sample_covers_every_request_variant() {
        let seen: std::collections::BTreeSet<&'static str> =
            every_request().iter().map(request_tag).collect();
        assert_eq!(
            seen,
            [
                "Alloc",
                "AllocChannel",
                "AllocEngineObject",
                "AllocSysmem",
                "AllocVaSpace",
                "CeCopy",
                "Control",
                "DescribeGuestRam",
                "ExportBacking",
                "ExportSurface",
                "FbRead",
                "Free",
                "MapGpuVa",
                "MapGuestRam",
                "RingDoorbell",
                "Schedule",
                "UnmapGpuVa",
                "UnmapGuestRam",
            ]
            .into_iter()
            .collect(),
            "the sample list and the variant list must be the same set"
        );
    }

    #[test]
    fn every_request_round_trips_with_its_txn() {
        for (i, request) in every_request().into_iter().enumerate() {
            let env = Envelope {
                txn: i as u64 + 1,
                request,
            };
            assert_eq!(Envelope::decode(&env.encode()), Ok(env));
        }
    }

    #[test]
    fn every_reply_round_trips() {
        let replies = [
            Reply::Handle(0xC1D0_0007),
            Reply::HandleAndToken(1, 2),
            Reply::Unit,
            Reply::Payload(vec![7; 100]),
            Reply::Va(0x7f00_0000),
            Reply::Surface(3),
            Reply::Failed(WireError::InsufficientPermissions),
            Reply::Failed(WireError::BadHandle(0xBAD)),
            Reply::Failed(WireError::NoMemory),
            Reply::Failed(WireError::Interrupted),
            Reply::Failed(WireError::Other(0x56)),
            // ★ BOTH arms of the fabricated-aperture reply. `covered: false` with no bytes
            // is a MISS, `covered: true` with bytes is a page — a wire that dropped the
            // flag would round-trip the second correctly and turn the first into an empty
            // page, which is the one answer this seam must never give.
            Reply::FbBytes {
                covered: false,
                bytes: Vec::new(),
            },
            Reply::FbBytes {
                covered: true,
                bytes: vec![0xA5; 4096],
            },
            // ★ The reply whose descriptor is NOT in these bytes. It round-trips as
            // geometry alone, which is the encoding property that matters: the fd travels
            // as ancillary data and nothing in the body names it.
            Reply::Backing {
                offset: 0,
                len: 0x20_0000,
                prot: PROT_READ_WRITE,
            },
            Reply::Backing {
                offset: 0x1000,
                len: 0x1000,
                prot: PROT_READ_ONLY,
            },
            Reply::Failed(WireError::NotExportableAsMemory(0xC1D0_0011)),
        ];
        for r in replies {
            assert_eq!(Reply::decode(&r.encode()), Ok(r));
        }
    }

    /// ★★ The two variants the child may not claim (module docs). Asserted by
    /// **exhaustion**: every `WireError` maps to an `RmError` that is neither, so there is
    /// no encoding of either — and the `match` below fails to compile if `WireError` grows
    /// a variant, which is the point.
    #[test]
    fn wire_errors_that_the_child_may_not_claim() {
        let iso = kayfabe_isolate::IsolateId::new(1, kayfabe_arch::ids::GpuId(0));
        for w in [
            WireError::InsufficientPermissions,
            WireError::BadHandle(1),
            WireError::NoMemory,
            WireError::Interrupted,
            WireError::Other(0),
            WireError::NotExportableAsMemory(1),
            WireError::GuestRamUnavailable,
        ] {
            match w.into_rm_error(iso) {
                RmError::Wedged => panic!("a child claimed it never answered"),
                RmError::ForeignHandle { .. } => panic!("a child claimed OUR gate fired"),
                _ => {}
            }
        }
    }

    /// ★★★ The **named boundary** survives the wire as itself, and is stamped with OUR
    /// namespace — not the child's.
    ///
    /// Both halves matter. If `NotExportableAsMemory` arrived as `Other`, a VMM could not
    /// tell *"the bytes are on the card"* from *"the host refused"*, and the whole point
    /// of naming the boundary would be lost at the last hop. If the handle carried the
    /// child's claimed namespace, a compromised isolate could name a **sibling's** object
    /// in a refusal the VMM logs and acts on.
    #[test]
    fn the_unexportable_boundary_keeps_its_name_and_our_namespace() {
        let ours = kayfabe_isolate::IsolateId::new(4, kayfabe_arch::ids::GpuId(1));
        let round_tripped =
            Reply::decode(&Reply::Failed(WireError::NotExportableAsMemory(0x77)).encode())
                .expect("decodes");
        let Reply::Failed(w) = round_tripped else {
            panic!("expected a failure reply");
        };
        match w.into_rm_error(ours) {
            RmError::NotExportableAsMemory { memory } => {
                assert_eq!(memory.isolate(), ours, "stamped with OUR namespace");
                assert_eq!(memory.raw(), 0x77);
            }
            other => panic!("the boundary lost its name on the wire: {other:?}"),
        }
    }

    /// ★★ Neither wire code may be defaulted. A `prot` byte the child does not recognise
    /// must **refuse**, and specifically must not fall through to read-write: that
    /// direction hands the guest a write it was never granted.
    #[test]
    fn an_unknown_protection_code_is_refused_rather_than_defaulted_to_writable() {
        assert_eq!(
            prot_from_code(PROT_READ_WRITE),
            Some(kayfabe_vmm::Prot::ReadWrite)
        );
        assert_eq!(
            prot_from_code(PROT_READ_ONLY),
            Some(kayfabe_vmm::Prot::ReadOnly)
        );
        for bad in [2u8, 3, 0xFF] {
            assert_eq!(prot_from_code(bad), None, "code {bad} must not decode");
        }
        for p in [kayfabe_vmm::Prot::ReadWrite, kayfabe_vmm::Prot::ReadOnly] {
            assert_eq!(prot_from_code(prot_code(p)), Some(p));
        }
    }

    /// A `BadHandle` is stamped with the namespace **we** asked on, so a compromised child
    /// cannot name another isolate's namespace.
    #[test]
    fn a_reported_bad_handle_is_stamped_with_our_namespace_not_the_childs() {
        let ours = kayfabe_isolate::IsolateId::new(4, kayfabe_arch::ids::GpuId(1));
        match WireError::BadHandle(0x99).into_rm_error(ours) {
            RmError::BadHandle(h) => {
                assert_eq!(h.isolate(), ours);
                assert_eq!(h.raw(), 0x99);
            }
            other => panic!("expected BadHandle, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_request_tag_is_a_named_refusal() {
        let mut body = 1u64.to_le_bytes().to_vec();
        body.push(200);
        assert_eq!(
            Envelope::decode(&body),
            Err(ProtoError::UnknownTag {
                what: "request",
                tag: 200
            })
        );
    }

    #[test]
    fn a_truncated_field_is_a_named_refusal() {
        let env = Envelope {
            txn: 1,
            request: Request::MapGpuVa {
                vas: 1,
                memory: 2,
                len: 3,
                at: 0x4000,
            },
        };
        let full = env.encode();
        assert_eq!(
            Envelope::decode(&full[..full.len() - 1]),
            // #102: `at` is the LAST field on the wire, so a one-byte truncation now
            // lands on it. That the refusal moved is the point — the placement address
            // is carried, and a frame missing it is refused by name rather than decoded
            // into a map at address zero.
            Err(ProtoError::Truncated { what: "map at" })
        );
    }

    /// ★ Trailing bytes are refused. A peer that appends is a peer we do not understand,
    /// and tolerating it is where a parser differential begins.
    #[test]
    fn trailing_bytes_are_refused_rather_than_ignored() {
        let mut body = Envelope {
            txn: 1,
            request: Request::AllocVaSpace,
        }
        .encode();
        body.push(0);
        assert_eq!(
            Envelope::decode(&body),
            Err(ProtoError::TrailingBytes { len: 1 })
        );
    }

    #[test]
    fn a_blob_longer_than_the_bound_is_refused_without_reading_it() {
        let mut body = 1u64.to_le_bytes().to_vec();
        body.push(1); // Alloc
        body.extend_from_slice(&0u64.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&(FRAME_MAX as u32 + 1).to_le_bytes());
        assert_eq!(
            Envelope::decode(&body),
            Err(ProtoError::Oversize {
                what: "alloc params",
                len: FRAME_MAX as u64 + 1,
            })
        );
    }

    #[test]
    fn engine_codes_round_trip_and_an_unknown_one_is_refused() {
        for e in [
            EngineKind::GrCompute,
            EngineKind::GrGraphics,
            EngineKind::Ce,
            EngineKind::NvEnc,
            EngineKind::NvDec,
            EngineKind::Other,
        ] {
            assert_eq!(engine_from_code(engine_code(e)), Some(e));
        }
        assert_eq!(engine_from_code(0), None, "0 must not default to an engine");
        assert_eq!(engine_from_code(7), None);
    }

    /// ★ The GR-1 property, stated as an assertion rather than a comment: no engine's wire
    /// code is zero, so a zero-initialised or truncated field cannot name a valid engine.
    /// The C's `engineType = 0` shipped a channel to the wrong runlist and cost a
    /// `cuCtxCreate 401`.
    #[test]
    fn no_engine_code_is_zero() {
        for e in [
            EngineKind::GrCompute,
            EngineKind::GrGraphics,
            EngineKind::Ce,
            EngineKind::NvEnc,
            EngineKind::NvDec,
            EngineKind::Other,
        ] {
            assert_ne!(engine_code(e), 0);
        }
    }

    #[test]
    fn a_frame_round_trips_over_a_byte_channel() {
        let mut chan: Vec<u8> = Vec::new();
        let env = Envelope {
            txn: 42,
            request: Request::Control {
                obj: 1,
                cmd: 2,
                payload: vec![9; 300],
            },
        };
        write_frame(&mut chan, &env.encode()).expect("write");
        let mut r = chan.as_slice();
        let mut buf = Vec::new();
        assert!(read_frame(&mut r, &mut buf).expect("read"));
        assert_eq!(Envelope::decode(&buf), Ok(env));
        assert!(!read_frame(&mut r, &mut buf).expect("eof"), "clean EOF");
    }

    /// ★★ A hostile length is refused before a byte of body is consumed — asserted by the
    /// channel still holding all of its bytes afterwards.
    #[test]
    fn a_hostile_frame_length_is_refused_without_consuming_the_body() {
        let mut chan = (FRAME_MAX as u32 + 1).to_le_bytes().to_vec();
        chan.extend_from_slice(&[0xAB; 16]);
        let mut r = chan.as_slice();
        let mut buf = Vec::new();
        let e = read_frame(&mut r, &mut buf).expect_err("must refuse");
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        assert_eq!(r.len(), 16, "the body must not have been consumed");
        assert!(buf.is_empty(), "nothing was allocated for it");
    }

    #[test]
    fn a_peer_that_dies_between_length_and_body_is_distinguished_from_a_clean_eof() {
        let mut chan = 32u32.to_le_bytes().to_vec();
        chan.extend_from_slice(&[0; 4]);
        let mut r = chan.as_slice();
        let mut buf = Vec::new();
        let e = read_frame(&mut r, &mut buf).expect_err("must refuse");
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }
}
