//! The **parent** side: the factory that spawns a sandboxed child per `(Proc, GpuId)`, the
//! pool of workers over its sockets, and the cancel seam that reaches a worker without
//! touching it.
//!
//! `l1_concurrency.md` §7.1–§7.3 and `l1_os_shell.md` §7.1–§7.5, made real. Everything here
//! has a `kayfabe_mocks` counterpart that the suite has been driving since L1-M1; where
//! this differs from the mock, the difference is a finding and is written down at the
//! point it occurs.
//!
//! ## ★ The descriptor contract with the child
//!
//! | fd | what |
//! |----|------|
//! | 3  | the control datagram socket (parent → child: interrupt requests) |
//! | 4  | ★ **deliberately vacant** — see below |
//! | 5.. | one request/reply stream per pool worker, in slot order |
//!
//! Numbers rather than a negotiation because an `exec`'d child has no other channel.
//!
//! ## ★★★ fd 4 is vacant, and that vacancy is a fix
//!
//! It used to carry the `/dev` `O_PATH` directory — the C's `NVKVM_DEV_DIRFD`, same number,
//! opened **here in the parent** and granted down. Measured against the real child, that
//! descriptor was a full host-filesystem escape: `openat(4, "../etc/shadow")` **opened**,
//! because `O_PATH` restricts enumeration and places no restriction at all on `..`.
//!
//! There is no bounded way for a parent to hand that grant down, so the grant is gone. The
//! child mints its own, from inside a `pivot_root`ed sandbox, via
//! [`kayfabe_linux_raw::sandbox::enter`] — which is where the whole argument lives. The
//! number stays reserved and unused so that a future edit re-adding a `/dev` grant collides
//! with a documented hole rather than quietly restoring the escape.
//!
//! ## ★★ Three things the real isolate does that the mock cannot
//!
//! 1. **Handles are minted by the child and stamped by the parent.** The wire carries a raw
//!    `u64`; [`ProxyRmBackend`] wraps it in *its own* [`IsolateId`]. A compromised child
//!    therefore cannot forge a handle into a sibling isolate's namespace — not because the
//!    protocol forbids it, but because the field does not exist on the wire.
//! 2. **Two isolates really do mint the same raw values.** Each child starts its own handle
//!    sequence, so proc A's third object and proc B's third object have the *identical*
//!    raw value and are unrelated live objects. That is what
//!    `kayfabe_isolate::HostHandle`'s docs say a real host does and what
//!    `MockRmBackend::check` had to be taught to imitate (`host_execution_plane.md` §2.1).
//!    Here it is free.
//! 3. **`RmError::Wedged` is produced, by a real EOF.** Its own rustdoc says *"no real
//!    backend returns it"*, which was true of a backend that is a function call. A backend
//!    that is a **socket** synthesises it the moment the reply channel closes — see
//!    [`ProxyRmBackend::call`].

use crate::export::ExportRegistry;
use crate::fdcross::read_frame_with_fds;
use crate::proto::{
    EXPORT_SOURCE_FABRICATED, EXPORT_SOURCE_HOST_DEVICE, Envelope, Reply, Request, WireError,
    engine_code, prot_code, prot_from_code, read_frame, write_frame,
};
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuVa};
use kayfabe_isolate::{
    CancelHandle, CancelReason, CancelSink, CeExecutor, CeSource, CeSubCopy, DEFAULT_POOL_WORKERS,
    ExportRequest, ExportSource, ExportedBacking, GuestRamGrant, GuestRamMapped, HostHandle,
    Isolate, IsolateFactory, IsolateId, RmBackend, RmError, Txn, Worker, WorkerId,
};
use kayfabe_linux_raw::{ChildSpec, FdGrant, ProgramImage, SandboxChild};
use kayfabe_vmm::SurfaceHandle;
use std::io::ErrorKind;
use std::os::fd::AsFd;
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::sync::{Arc, Mutex, OnceLock};

// =====================================================================================
// The embedded isolate
// =====================================================================================

/// ★★★ The isolate binary, compiled into this one.
///
/// `build.rs` produces it: a **static** `x86_64-unknown-linux-musl` build of this package's
/// `kayfabe-isolate` binary. See that file for the build-ordering argument and for the one
/// reviewed opt-out that makes these bytes empty.
static ISOLATE_IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kayfabe-isolate.image"));

/// The embedded isolate's bytes, for a test that has to assert what got embedded.
#[must_use]
pub fn embedded_isolate_bytes() -> &'static [u8] {
    ISOLATE_IMAGE
}

/// ★ The image, published **once per process** into a sealed `memfd`.
///
/// Once, because the seal is what makes the bytes immutable and a per-spawn republication
/// would be a per-spawn window in which they are not. Each spawn takes its own duplicate of
/// the descriptor, so one child's exit cannot invalidate the next one's image.
fn embedded_image() -> Result<&'static Arc<ProgramImage>, String> {
    static IMAGE: OnceLock<Result<Arc<ProgramImage>, String>> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            ProgramImage::from_bytes(c"kayfabe-isolate", ISOLATE_IMAGE)
                .map(Arc::new)
                .map_err(|e| format!("the embedded isolate image could not be published: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// fd number of the control datagram socket in the child.
pub const CONTROL_FD: i32 = 3;
/// ★ The number the C parks `NVKVM_DEV_DIRFD` on, **reserved and never granted** here. See
/// the module docs: a parent-opened `/dev` descriptor is an escape, and the vacancy is the
/// fix rather than an oversight.
pub const RESERVED_NEVER_GRANTED_FD: i32 = 4;
/// ★ fd number of the **park witness** write end, granted **only** when `--park` is armed.
///
/// Test-support scaffolding: see [`crate::loopback::LoopbackShared`]'s `park_witness` field for
/// why a duration cannot stand in for it. ⊘ A production isolate never receives this grant, so
/// this number is simply an unopened descriptor there — reserved, not silently reused.
pub const PARK_WITNESS_FD: i32 = 5;
/// ★★★★★ fd number of the **guest-RAM descriptor**, granted only when the VMM has one.
///
/// A **fixed** number, and that is the whole design rather than tidiness
/// (`mode2_isolate_memory_boundary.md` §2): the seccomp filter that will later deny
/// `read`/`write`/`lseek`/`ioctl`/`close` on this descriptor, and deny `dup`/`fcntl(F_DUPFD*)`
/// so the number cannot move, has to **hardcode a number**. A descriptor that arrived
/// per-request would have none to hardcode.
///
/// ⊘ Like [`PARK_WITNESS_FD`], the number is **reserved rather than reused** when no grant
/// is made: an isolate whose VM was launched without a shared memory backing simply has an
/// unopened descriptor here, so the layout does not depend on a deployment choice.
pub const GUEST_RAM_FD: i32 = 6;
/// fd number of pool worker 0's stream in the child; worker *n* is `WORKER_FD_BASE + n`.
pub const WORKER_FD_BASE: i32 = 7;

// ★ The descriptor contract, checked at COMPILE time rather than by a test. It is a
// relationship between three constants, so a runtime assertion could only ever be
// constant-folded — and a "test" a compiler can prove is not a test. A future edit that
// collides two grants fails the build.
const _: () = {
    assert!(CONTROL_FD >= 3, "grants start above the standard streams");
    assert!(CONTROL_FD != RESERVED_NEVER_GRANTED_FD);
    assert!(
        PARK_WITNESS_FD > RESERVED_NEVER_GRANTED_FD,
        "the park witness comes after the reserved hole"
    );
    assert!(
        GUEST_RAM_FD > PARK_WITNESS_FD,
        "guest RAM comes after the reserved hole and the park witness"
    );
    assert!(
        WORKER_FD_BASE > GUEST_RAM_FD,
        "workers come after the reserved hole, the park witness AND guest RAM"
    );
    assert!(
        RESERVED_NEVER_GRANTED_FD == 4,
        "the C's NVKVM_DEV_DIRFD value, kept vacant"
    );
};

/// Which RM the child is to speak to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmMode {
    /// The real driver: `/dev/nvidiactl` plus `/dev/nvidia<gpu>`.
    Real,
    /// ★ **The transport's fixture, and it is NOT a model of RM.**
    ///
    /// A host with no GPU still has to exercise the thing this crate is: a real process, a
    /// real socket, a real fd table, real blocking, real signal-driven cancellation. So the
    /// child can be told to serve verbs from an in-process table whose *parking* is a real
    /// `read(2)` on a real pipe — which means a real `SIGUSR1` really returns `EINTR`
    /// through the whole stack.
    ///
    /// It deliberately models **one** RM semantic and no others: the per-client
    /// serialisation `rm_concurrency_semantics` measured. Everything else it answers
    /// promptly, because pretending to model a driver is how a fixture becomes the thing
    /// the design is validated against (`host_execution_plane.md` §5).
    Loopback,
}

impl RmMode {
    fn as_arg(self) -> &'static str {
        match self {
            RmMode::Real => "real",
            RmMode::Loopback => "loopback",
        }
    }

    /// Parse the child's `--rm` argument. `None` for anything else — an unrecognised mode
    /// must not fall back to `Real`, which would put a GPU-free test on a GPU.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "real" => Some(RmMode::Real),
            "loopback" => Some(RmMode::Loopback),
            _ => None,
        }
    }
}

// =====================================================================================
// The cancel seam
// =====================================================================================

#[derive(Debug, Default)]
struct CancelState {
    /// The txn this slot is currently checked out under, if any.
    txn: Option<u64>,
    /// The reason a cancel was *delivered* for. Not yet what the verb observed.
    armed: Option<CancelReason>,
    /// What the verb actually observed — set only when the isolate answered `Interrupted`.
    observed: Option<CancelReason>,
}

/// The real out-of-band cancel seam (`l1_os_shell.md` §7.1/§7.2).
///
/// Holds **no** reference to the [`Worker`] or its backend, exactly as the port requires:
/// the thread that could cancel is never the thread that holds the worker, because the
/// holder is blocked inside the verb. What it holds is the two descriptors that can reach a
/// blocked thread from outside — the control socket (interrupt) and the worker's own reply
/// stream (abandon).
#[derive(Debug)]
pub struct HostCancelSink {
    worker: WorkerId,
    /// The worker's reply stream, for §7.5's abandon: `shutdown` makes the blocked
    /// `read(2)` return zero.
    reply: Arc<UnixStream>,
    /// The isolate-wide control socket, for §7.2's interrupt.
    control: Arc<UnixDatagram>,
    state: Mutex<CancelState>,
}

/// The control message. A fixed 13-byte datagram over `SOCK_DGRAM`, so it is atomic and
/// self-delimiting by the kernel rather than by a length field we would have to trust.
///
/// The C sent its interrupt on the **same** socket as everything else and called it
/// out-of-band because it expects no reply (`C: src/qemu/nvkvm_isolate.c:1447-1477`). That
/// works, but it means a cancel queues behind whatever request is in the socket. A separate
/// datagram socket costs one descriptor per isolate and makes "out of band" mean it.
const CONTROL_MSG_LEN: usize = 13;

fn reason_code(reason: CancelReason) -> u8 {
    match reason {
        CancelReason::ProcExit => 1,
        CancelReason::DeviceReset => 2,
        CancelReason::Watchdog => 3,
        CancelReason::GuestSignal => 4,
    }
}

/// Decode a control datagram: `(worker, txn, reason)`. `None` for anything malformed — the
/// child refuses rather than guesses, because a mis-decoded cancel lands on an innocent
/// operation, which is the sharpest bug in this area (`Txn`'s own docs).
#[must_use]
pub fn decode_control(bytes: &[u8]) -> Option<(u32, u64, u8)> {
    if bytes.len() != CONTROL_MSG_LEN {
        return None;
    }
    let worker = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let txn = u64::from_le_bytes([
        bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
    ]);
    let reason = bytes[12];
    (1..=4).contains(&reason).then_some((worker, txn, reason))
}

impl HostCancelSink {
    fn begin(&self, txn: u64) {
        let mut s = self.state.lock().expect("cancel state");
        s.txn = Some(txn);
        s.armed = None;
        s.observed = None;
    }

    fn end(&self) {
        self.state.lock().expect("cancel state").txn = None;
    }

    fn current_txn(&self) -> Option<u64> {
        self.state.lock().expect("cancel state").txn
    }

    /// The verb came back `Interrupted`: promote the *delivered* reason to the *observed*
    /// one. Deliberately not done at delivery — a cancel that was sent but that the host
    /// call never noticed is a request that lost, and reporting it as the cause of a verb
    /// that succeeded would be a lie in the one place §7.3 says must carry the truth.
    fn promote_observed(&self) {
        let mut s = self.state.lock().expect("cancel state");
        s.observed = s.armed;
    }
}

impl CancelSink for HostCancelSink {
    fn deliver(&self, txn: Txn, reason: CancelReason) -> bool {
        // The staleness check and the syscall are deliberately NOT one critical section:
        // the lock is dropped before the send. §7.1 forbids a syscall under a lock and the
        // rule has no exception for a small one.
        {
            let mut s = self.state.lock().expect("cancel state");
            if s.txn != Some(txn.0) {
                return false;
            }
            s.armed = Some(reason);
        }
        let mut msg = [0u8; CONTROL_MSG_LEN];
        msg[..4].copy_from_slice(&self.worker.0.to_le_bytes());
        msg[4..12].copy_from_slice(&txn.0.to_le_bytes());
        msg[12] = reason_code(reason);
        // A failed send means the child is gone, which the reply path will report as a
        // wedge. Reporting it here as "not armed" would be worse: the caller would believe
        // the txn was already complete.
        let _ = self.control.send(&msg);
        true
    }

    fn abandon(&self, txn: Txn) -> bool {
        {
            let s = self.state.lock().expect("cancel state");
            if s.txn != Some(txn.0) {
                return false;
            }
        }
        // ★ §7.5's escape, and it is one syscall: shutting down our READ half makes the
        // requester's blocked `read` return zero immediately. Safe only because the caller
        // kills the slot in the same act, so no future reader of this stream exists.
        let _ = self.reply.shutdown(std::net::Shutdown::Read);
        true
    }

    fn observed(&self) -> Option<CancelReason> {
        self.state.lock().expect("cancel state").observed
    }
}

// =====================================================================================
// The backend — one socket, one verb at a time
// =====================================================================================

/// The parent half of one pool worker: a [`RmBackend`] whose implementation is a socket.
///
/// Strictly single-in-flight, which the borrow checker gives for free — a [`Worker`] is
/// reached only by `&mut`, and this is inside one.
#[derive(Debug)]
pub struct ProxyRmBackend {
    isolate: IsolateId,
    sock: Arc<UnixStream>,
    cancel: Arc<HostCancelSink>,
    buf: Vec<u8>,
    /// ★ The isolate's export registry, shared by every worker in its pool — see
    /// [`crate::export`] for why the scope is the isolate and not the worker.
    exports: Arc<ExportRegistry>,
}

impl ProxyRmBackend {
    /// ★ One request, one reply.
    ///
    /// Every failure that is not a *reported* one becomes [`RmError::Wedged`], and the
    /// reasoning is §7.2's: **the requester never abandons the reply**. So if this returns
    /// at all without a reply, the channel is finished — either the watchdog abandoned it
    /// (§7.5), or the child died, or the child sent something we do not understand. In all
    /// three the observable is identical (*no answer, and the unwind cannot run on this
    /// worker*), and that observable is exactly what `Wedged` names. Splitting them into
    /// separate errors would invite a caller to treat one as retryable.
    fn call(&mut self, request: Request) -> Result<Reply, RmError> {
        let txn = self.cancel.current_txn().unwrap_or(0);
        let body = Envelope { txn, request }.encode();
        let mut sock = &*self.sock;
        if write_frame(&mut sock, &body).is_err() {
            return Err(RmError::Wedged);
        }
        match read_frame(&mut sock, &mut self.buf) {
            Ok(true) => match Reply::decode(&self.buf) {
                Ok(reply) => Ok(reply),
                Err(_) => Err(RmError::Wedged),
            },
            // A clean end of stream: the abandon landed, or the child exited.
            Ok(false) => Err(RmError::Wedged),
            Err(e) if e.kind() == ErrorKind::Interrupted => Err(RmError::Wedged),
            Err(_) => Err(RmError::Wedged),
        }
    }

    /// Lift a reply, or the failure it reported.
    fn lift(&self, reply: Reply) -> Result<Reply, RmError> {
        match reply {
            Reply::Failed(e) => {
                if e == WireError::Interrupted {
                    self.cancel.promote_observed();
                }
                Err(e.into_rm_error(self.isolate))
            }
            other => Ok(other),
        }
    }

    fn handle(&mut self, request: Request) -> Result<HostHandle, RmError> {
        let reply = self.call(request)?;
        match self.lift(reply)? {
            Reply::Handle(raw) => Ok(HostHandle::new(self.isolate, raw)),
            _ => Err(RmError::Wedged),
        }
    }

    fn unit(&mut self, request: Request) -> Result<(), RmError> {
        let reply = self.call(request)?;
        match self.lift(reply)? {
            Reply::Unit => Ok(()),
            _ => Err(RmError::Wedged),
        }
    }

    /// ★★★ The **one** call that reads with a descriptor allowance
    /// (`isolate_vmm_fd_crossing.md` §12).
    ///
    /// Everything about this differs from [`ProxyRmBackend::call`] in exactly one way —
    /// `max_fds = 1` instead of the fd-free reader — and that one way is the whole
    /// protocol policy §6 describes. Every other verb's reply is read by `read_frame`,
    /// which has **no** control buffer at all, so a child that attaches a descriptor to an
    /// `Alloc` reply has it dropped by the kernel and never reaches this process's
    /// descriptor table. The allowance is per call because that is what makes it
    /// *testable*: undersize it and watch the refusal fire.
    ///
    /// ⚠ **The frame is refused whole if the count is wrong**, never served from what
    /// fitted — a peer must not get to choose which of its descriptors we act on.
    fn call_for_backing(&mut self, request: Request) -> Result<ExportedBacking, RmError> {
        let txn = self.cancel.current_txn().unwrap_or(0);
        let body = Envelope { txn, request }.encode();
        let mut sock = &*self.sock;
        if write_frame(&mut sock, &body).is_err() {
            return Err(RmError::Wedged);
        }
        let mut fds = Vec::new();
        // ★ A refusal from here leaves `fds` owned, so every path out of this function —
        // including the ones that refuse below — closes whatever arrived, by `Drop`.
        let Ok(true) = read_frame_with_fds(self.sock.as_fd(), &mut self.buf, &mut fds, 1) else {
            return Err(RmError::Wedged);
        };
        let Ok(reply) = Reply::decode(&self.buf) else {
            return Err(RmError::Wedged);
        };
        let (offset, len, prot) = match self.lift(reply)? {
            Reply::Backing { offset, len, prot } => (offset, len, prot),
            // ★ A reply of any other shape must not leave descriptors adopted. `fds` is
            // dropped on this path, which closes them — the C's R2-M1 sweep, had for free.
            _ => return Err(RmError::Wedged),
        };
        // ★★ Exactly one, and the count is checked HERE rather than trusted from the
        // allowance: `read_frame_with_fds` bounds the maximum, and a child that attaches
        // *none* to a `Backing` reply is claiming a backing it did not hand over.
        let Ok([fd]) = <[_; 1]>::try_from(fds) else {
            return Err(RmError::Wedged);
        };
        let Some(prot) = prot_from_code(prot) else {
            return Err(RmError::Wedged);
        };
        // ★★★ The kind check. `adopt` takes the descriptor by value, so a child answering
        // with a character device has it REFUSED and CLOSED here, before anything in this
        // process can `mmap` or `ioctl` it. This is the enforcement point for the property
        // the verb exists for, and it is deliberately independent of the child's own
        // refusal: a compromised isolate is inside the threat model.
        let Ok(token) = self.exports.adopt(fd, self.isolate) else {
            return Err(RmError::Wedged);
        };
        Ok(ExportedBacking {
            token,
            offset,
            len,
            prot,
        })
    }
}

impl RmBackend for ProxyRmBackend {
    fn alloc(
        &mut self,
        parent: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError> {
        self.handle(Request::Alloc {
            parent: parent.raw(),
            class: class.0,
            params: params.to_vec(),
        })
    }

    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError> {
        self.handle(Request::AllocVaSpace)
    }

    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        self.handle(Request::AllocSysmem { len })
    }

    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError> {
        let reply = self.call(Request::AllocChannel {
            vas: vas.raw(),
            engine: engine_code(engine),
        })?;
        match self.lift(reply)? {
            Reply::HandleAndToken(h, t) => Ok((HostHandle::new(self.isolate, h), t)),
            _ => Err(RmError::Wedged),
        }
    }

    fn alloc_engine_object(
        &mut self,
        chan: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError> {
        self.handle(Request::AllocEngineObject {
            chan: chan.raw(),
            class: class.0,
            params: params.to_vec(),
        })
    }

    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError> {
        self.unit(Request::Schedule { chan: chan.raw() })
    }

    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        self.unit(Request::Free { obj: obj.raw() })
    }

    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<(), RmError> {
        let reply = self.call(Request::Control {
            obj: obj.raw(),
            cmd: cmd.0,
            payload: payload.to_vec(),
        })?;
        match self.lift(reply)? {
            Reply::Payload(out) if out.len() == payload.len() => {
                payload.copy_from_slice(&out);
                Ok(())
            }
            // ★ A control whose written-back payload changed LENGTH is refused. The caller
            // supplied a fixed-size parameter block; a child that returns a different
            // length is either confused or probing, and copying a prefix would hand the
            // caller a half-updated struct that looks successful.
            Reply::Payload(_) => Err(RmError::Wedged),
            _ => Err(RmError::Wedged),
        }
    }

    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, RmError> {
        let reply = self.call(Request::MapGpuVa {
            vas: vas.raw(),
            memory: memory.raw(),
            len,
            at: at.0,
        })?;
        match self.lift(reply)? {
            Reply::Va(va) => Ok(va),
            _ => Err(RmError::Wedged),
        }
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        self.unit(Request::UnmapGpuVa {
            vas: vas.raw(),
            gpu_va,
        })
    }

    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError> {
        self.unit(Request::RingDoorbell { token: host_token })
    }

    fn ce_copy(&mut self, vas: HostHandle, sub: CeSubCopy) -> Result<(), RmError> {
        let (src, src_is_const) = match sub.src {
            CeSource::Address(a) => (a, 0u8),
            CeSource::Constant(c) => (u64::from(c), 1u8),
        };
        self.unit(Request::CeCopy {
            vas: vas.raw(),
            dst: sub.dst,
            src,
            len: sub.len,
            src_is_const,
            by_ours: match sub.by {
                CeExecutor::HostCe => 0,
                CeExecutor::Ours => 1,
            },
        })
    }

    /// ★★★ #102 stage C3. The reply's `covered` flag is carried through **unchanged**:
    /// the parent does not get to reinterpret "the aperture does not reach there" as an
    /// error or as zeros, because the walker's `MISS = FAULT` rule is stated over exactly
    /// this distinction.
    ///
    /// A short reply is a **protocol** failure, not a partial read. `buf` is left
    /// untouched in that case, so a caller that ignored the error cannot then decode
    /// half-stale bytes.
    fn fb_read(&mut self, phys: u64, buf: &mut [u8]) -> Result<bool, RmError> {
        let reply = self.call(Request::FbRead {
            phys,
            len: buf.len() as u64,
        })?;
        match self.lift(reply)? {
            Reply::FbBytes {
                covered: false,
                bytes,
            } if bytes.is_empty() => Ok(false),
            Reply::FbBytes {
                covered: true,
                bytes,
            } if bytes.len() == buf.len() => {
                buf.copy_from_slice(&bytes);
                Ok(true)
            }
            _ => Err(RmError::Wedged),
        }
    }

    /// ★★★ Decision (b), on the wire (`isolate_vmm_fd_crossing.md` §12).
    ///
    /// The request carries no descriptor and the reply carries exactly one — a `memfd`,
    /// checked against the kernel before it is reachable. What the VMM ends up holding is
    /// a token into [`ExportRegistry`], which yields a backing it can `mmap` and install;
    /// it never holds anything with an RM `ioctl` handler behind it.
    fn export_backing(&mut self, want: ExportRequest) -> Result<ExportedBacking, RmError> {
        let (source, memory) = match want.source {
            ExportSource::Fabricated => (EXPORT_SOURCE_FABRICATED, 0),
            ExportSource::HostDeviceMemory { memory } => (EXPORT_SOURCE_HOST_DEVICE, memory.raw()),
        };
        self.call_for_backing(Request::ExportBacking {
            source,
            memory,
            len: want.len,
            prot: prot_code(want.prot),
        })
    }

    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        let reply = self.call(Request::ExportSurface {
            memory: memory.raw(),
        })?;
        match self.lift(reply)? {
            Reply::Surface(s) => Ok(SurfaceHandle(s)),
            _ => Err(RmError::Wedged),
        }
    }

    /// ★★★★★ Carry the VMM's guest-RAM grant to the child.
    ///
    /// ⊘ **No descriptor rides this frame**, and the ordinary `write_frame`/`read_frame`
    /// pair is used rather than the fd-carrying twins. The guest-RAM `memfd` crossed at
    /// SPAWN, on [`GUEST_RAM_FD`] — see [`crate::guestram`] for why a fixed number is the
    /// design rather than a convenience. So `max_fds` stays **zero** on this reply, exactly
    /// as it is for every request but `ExportBacking`, and a child that attached one anyway
    /// has it closed by the kernel and the frame refused.
    fn map_guest_ram(&mut self, grant: GuestRamGrant) -> Result<GuestRamMapped, RmError> {
        let reply = self.call(Request::MapGuestRam {
            offset: grant.offset(),
            len: grant.len(),
            prot: prot_code(grant.prot()),
        })?;
        match self.lift(reply)? {
            // ★ The isolate is stamped HERE, from the connection we asked on — never taken
            // from the wire. A child cannot name another isolate's namespace even by lying,
            // which is the same rule `WireError::into_rm_error` applies to a `BadHandle`.
            Reply::Handle(raw) => Ok(GuestRamMapped {
                region: HostHandle::new(self.isolate, raw),
                len: grant.len(),
            }),
            _ => Err(RmError::Wedged),
        }
    }

    fn unmap_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<(), RmError> {
        self.unit(Request::UnmapGuestRam {
            region: mapped.region.raw(),
            len: mapped.len,
        })
    }

    /// ★★★★★ Ask the child to describe its guest-RAM mapping to RM.
    ///
    /// ★ The returned handle is stamped with **this connection's** isolate, exactly as
    /// [`Self::map_guest_ram`] stamps the mapping's name — a child cannot name another
    /// isolate's namespace even by lying, and the object this mints is one
    /// [`kayfabe_isolate::Worker::execute`]'s foreign-handle gate must be able to judge.
    fn describe_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<HostHandle, RmError> {
        let reply = self.call(Request::DescribeGuestRam {
            region: mapped.region.raw(),
            len: mapped.len,
        })?;
        match self.lift(reply)? {
            Reply::Handle(raw) => Ok(HostHandle::new(self.isolate, raw)),
            _ => Err(RmError::Wedged),
        }
    }
}

// =====================================================================================
// The isolate
// =====================================================================================

#[derive(Debug)]
enum Slot {
    Idle(Box<Worker>),
    Busy,
    Dead,
}

/// One sandboxed child process per `(Proc, GpuId)`, with a bounded pool of workers over its
/// sockets.
#[derive(Debug)]
pub struct HostIsolate {
    id: IsolateId,
    /// `None` when the spawn failed — see [`HostIsolate::spawn_error`].
    child: Option<SandboxChild>,
    slots: Vec<Slot>,
    cancels: Vec<Arc<HostCancelSink>>,
    next_txn: u64,
    retired: bool,
    spawn_error: Option<String>,
    /// ★★★ The backings this isolate handed up, indexed by the token
    /// `RmBackend::export_backing` returned. **This is what the VMM installs from** — see
    /// [`HostIsolate::exports`].
    exports: Arc<ExportRegistry>,
    /// ★ Read end of the park witness — `Some` only when this isolate was spawned with a
    /// `--park` verb armed. See [`HostIsolate::wait_for_park`].
    park_witness: Option<std::io::PipeReader>,
    /// ★ The parent's OWN write end of the same pipe, kept solely so a watchdog can unblock
    /// [`HostIsolate::wait_for_park`] with a distinguishable byte.
    ///
    /// ⊘ Without it a broken witness would make the wait **hang**, and a hang wedges CI
    /// instead of failing it — the failure shape this repo has already been bitten by three
    /// times. A bound that produces a *named* error is the whole point.
    park_deadline: Option<std::io::PipeWriter>,
}

impl HostIsolate {
    /// ★★★ Block until this isolate's parked verb has **actually parked**.
    ///
    /// The fact a caller needs before acting on a wedged requester is *"the verb is parked"*,
    /// and the fact a timeout gives is *"no reply has arrived yet"*. The second is strictly
    /// weaker — it is also true before the chain has started — so a test that wants the first
    /// and waits for the second is betting on a host round trip. That bet is what made
    /// `abandon_releases_a_wedged_requester_with_wedged` flake ~0.5 % of the time, and it
    /// fails **20/20** when the duration is shortened to zero.
    ///
    /// This reads the byte the child writes immediately before its blocking `read`, so on
    /// return every earlier verb in the chain has already completed.
    ///
    /// ## `within` is a BOUND, not the thing being waited on
    ///
    /// ⚠ The distinction that this whole change is about survives here and must not be
    /// blurred. `within` never decides the answer on a healthy run: the wait ends when the
    /// **park happens**, however long that takes. `within` exists only so that a *broken*
    /// witness — a child that never announces — produces a named error instead of a hang,
    /// because a hang wedges CI rather than failing it. Pick it generously; it is a
    /// diagnostic ceiling, not a synchronisation device. If shortening it changes whether a
    /// green test is green, something else is wrong.
    ///
    /// # Errors
    /// The isolate was spawned without a park armed, the bound elapsed, or the read failed.
    ///
    /// # Panics
    /// Never.
    pub fn wait_for_park(&mut self, within: std::time::Duration) -> Result<(), String> {
        use std::io::{Read as _, Write as _};
        let deadline = self
            .park_deadline
            .as_ref()
            .ok_or("this isolate was spawned without a park armed, so nothing will ever park")?
            .try_clone()
            .map_err(|e| format!("duplicating the park deadline writer: {e}"))?;
        let reader = self
            .park_witness
            .as_mut()
            .ok_or("this isolate was spawned without a park armed, so nothing will ever park")?;

        // ★ The watchdog writes into the SAME pipe, so the blocking read below is what gets
        // unblocked — no non-blocking mode, no signal, no second descriptor to poll. It is
        // told to stand down the moment the real byte arrives, so the healthy path leaves no
        // thread sleeping and no stale byte behind.
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let watchdog = std::thread::spawn(move || {
            if stop_rx.recv_timeout(within).is_err() {
                let _ = (&mut &deadline).write_all(b"T");
            }
        });

        let mut byte = [0u8; 1];
        let read = reader.read_exact(&mut byte);
        let _ = stop_tx.send(());
        let _ = watchdog.join();

        match read {
            Ok(()) if byte == *b"P" => Ok(()),
            Ok(()) if byte == *b"T" => Err(format!(
                "★ no park was announced within {within:?}. The child either never reached the \
                 parked verb or no longer announces it — this is NOT a reason to lengthen the \
                 bound, which never decides a healthy run"
            )),
            Ok(()) => Err(format!("unknown park witness byte {byte:?}")),
            Err(e) => Err(format!("waiting for the park witness: {e}")),
        }
    }

    /// ★ Why the spawn failed, if it did.
    ///
    /// **A finding about the port, not about this type.**
    /// [`IsolateFactory::spawn`] returns `Box<dyn Isolate>` with **no error channel**, so a
    /// real spawn failure — a missing isolate binary, an exhausted descriptor table, a
    /// driver that is not loaded — has nowhere to go. The only representable outcome is a
    /// **stillborn** isolate: retired at birth, every slot dead, `checkout` returning
    /// `None`. That is correct behaviour (the core's backpressure path handles it) but it
    /// is indistinguishable from *"the pool is saturated"*, which is a transient condition
    /// and this is not.
    ///
    /// So the reason is recorded here rather than lost, and a composition root should check
    /// it at realize — the §4.4.1 pattern for a deployment fact no type can carry.
    #[must_use]
    pub fn spawn_error(&self) -> Option<&str> {
        self.spawn_error.as_deref()
    }

    /// The child's process id, or `None` for a stillborn isolate.
    #[must_use]
    pub fn child_pid(&self) -> Option<u32> {
        self.child.as_ref().map(SandboxChild::pid)
    }

    fn stillborn(id: IsolateId, pool: usize, why: String) -> Self {
        HostIsolate {
            id,
            child: None,
            slots: (0..pool).map(|_| Slot::Dead).collect(),
            cancels: Vec::new(),
            next_txn: 0,
            retired: true,
            spawn_error: Some(why),
            exports: Arc::new(ExportRegistry::new()),
            park_witness: None,
            park_deadline: None,
        }
    }

    /// ★★★ **What this isolate handed the VMM** — the registry a memslot install reads.
    ///
    /// The one accessor on the VMM's side of decision (b). It yields descriptors by
    /// duplication and never lends the registry's own, so an installer may hold a backing
    /// across this isolate's death — which it must, because a memslot outlives the call
    /// that installed it.
    ///
    /// ⊘ Note what it is NOT: there is no accessor here that yields a *device* descriptor,
    /// and there is nothing to add one to — [`ExportRegistry::adopt`] refuses anything that
    /// is not a regular file, so the registry cannot come to contain one.
    #[must_use]
    pub fn exports(&self) -> &Arc<ExportRegistry> {
        &self.exports
    }
}

impl Isolate for HostIsolate {
    fn id(&self) -> IsolateId {
        self.id
    }

    fn pool_size(&self) -> usize {
        self.slots.len()
    }

    fn idle_workers(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s, Slot::Idle(_)))
            .count()
    }

    fn checkout(&mut self) -> Option<Worker> {
        if self.retired {
            return None;
        }
        let i = self.slots.iter().position(|s| matches!(s, Slot::Idle(_)))?;
        match std::mem::replace(&mut self.slots[i], Slot::Busy) {
            Slot::Idle(w) => {
                self.next_txn += 1;
                let txn = self.next_txn;
                self.cancels[i].begin(txn);
                let mut w = *w;
                w.begin_txn(Txn(txn));
                Some(w)
            }
            _ => unreachable!("position() selected an Idle slot"),
        }
    }

    fn checkin(&mut self, worker: Worker) {
        let i = worker.id().0 as usize;
        if let Some(c) = self.cancels.get(i) {
            c.end();
        }
        match self.slots.get(i) {
            Some(Slot::Dead) | None => drop(worker),
            _ => self.slots[i] = Slot::Idle(Box::new(worker)),
        }
    }

    fn checked_out(&self) -> Vec<WorkerId> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Slot::Busy))
            .map(|(i, _)| WorkerId(i as u32))
            .collect()
    }

    fn cancel_handle(&self, worker: WorkerId) -> Option<CancelHandle> {
        // Keyed on "is a txn outstanding", NOT on `Slot::Busy` — `worker_died` turns a busy
        // slot dead while its requester is still parked inside the verb, and that requester
        // is exactly the one §7.5 must release.
        let i = worker.0 as usize;
        let sink = self.cancels.get(i)?;
        let txn = sink.current_txn()?;
        Some(CancelHandle::new(
            self.id,
            worker,
            Txn(txn),
            Arc::clone(sink) as Arc<dyn CancelSink>,
        ))
    }

    fn worker_died(&mut self, worker: WorkerId) -> bool {
        match self.slots.get_mut(worker.0 as usize) {
            Some(slot) => {
                *slot = Slot::Dead;
                true
            }
            None => false,
        }
    }

    fn in_flight(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s, Slot::Busy))
            .count()
    }

    fn retire(&mut self) {
        self.retired = true;
    }

    fn is_retired(&self) -> bool {
        self.retired
    }

    /// ★★★ **E1 — gap 7 closed at the seam this crate is on the wrong side of.**
    ///
    /// [`HostIsolate::spawn_error`] has recorded the reason since the type existed, and
    /// its own docs said *"a composition root should check it at realize"* — which no
    /// composition root could, because the core holds `dyn Isolate` and this accessor is
    /// on the concrete type. Answering it here is what makes the sentence reachable.
    ///
    /// ⊘ **`SpawnFailed` and never `NoPlane`.** This factory was *asked* for a real
    /// plane. Whether the cause was a build with no embedded image, a refused `clone`, or
    /// a child whose RM bring-up handshake failed, something was attempted on this host
    /// and did not work — which is precisely the half an operator must be able to
    /// separate from a deliberately plane-less build.
    ///
    /// ⊘ And it is `None` for a **live** isolate even after [`Isolate::retire`]: an
    /// ordinary teardown is not a refusal to be investigated, and reporting it as one
    /// would make every clean shutdown print a failure.
    fn refusal(&self) -> Option<kayfabe_isolate::IsolateRefusal<'_>> {
        self.spawn_error
            .as_deref()
            .map(|why| kayfabe_isolate::IsolateRefusal {
                kind: kayfabe_isolate::RefusalKind::SpawnFailed,
                why,
            })
    }
}

impl Drop for HostIsolate {
    /// ★ §7.0's garbage collector, and the reason `IsolateBox` asserts R1 on the drop side.
    ///
    /// Dropping the sockets makes every worker thread in the child see EOF and exit;
    /// dropping the child kills and reaps it, at which point the kernel frees **the entire
    /// RM object tree under this isolate's client** — every VAS, channel, mapping and
    /// surface, whether or not we knew about it. This is the one drop in the workspace that
    /// really is `waitpid`, which is why the port's `IsolateBox` exists.
    fn drop(&mut self) {
        self.slots.clear();
        self.cancels.clear();
    }
}

// =====================================================================================
// The factory
// =====================================================================================

/// Spawns [`HostIsolate`]s.
#[derive(Debug)]
pub struct HostIsolateFactory {
    /// ★★★ The isolate this factory spawns, as bytes in a sealed `memfd`. **Not a path.**
    ///
    /// There is no constructor that takes one, no environment variable that names one, and
    /// no directory that is searched — see the crate's `build.rs`. `Err` here is a build that
    /// embedded no image; it produces stillborn isolates that name why, exactly as a failed
    /// spawn does.
    image: Result<&'static Arc<ProgramImage>, String>,
    pool: usize,
    rm: RmMode,
    park: crate::loopback::ParkVerb,
    /// Every id this factory was asked for, in order — the isolate-per-`(Proc, GpuId)`
    /// witness, matching `MockIsolateFactory::spawned` so a test can assert the same
    /// property against either.
    ///
    /// ⊘ Behind a `Mutex` because [`IsolateFactory::spawn`] takes `&self` (R1: a spawn
    /// runs with zero locks held, so the factory must be reachable without the device
    /// lock). ★ The lock is taken **around the push and released before `build_isolate`**
    /// — holding it across the spawn would be the very inversion the `&self` signature
    /// exists to prevent, and `kayfabe_util::leafwitness` is blind to it. Read it with
    /// [`HostIsolateFactory::spawned`].
    spawned: std::sync::Mutex<Vec<IsolateId>>,
    /// ★★★ The guest-RAM descriptor every isolate this factory spawns is granted, and how
    /// many bytes it holds — or `None` when the VM was not launched with a shared memory
    /// backing.
    ///
    /// ⊘ The **length comes from the VMM**, which is why it is stored beside the descriptor
    /// rather than derived in the child. The extent of guest RAM is a fact the VMM owns;
    /// an isolate that re-derived it with an `lseek` would give the one number the whole
    /// authorization is bounded by a second source of truth.
    guest_ram: Option<(std::sync::Arc<OwnedFd>, u64)>,
}

impl HostIsolateFactory {
    /// A factory that spawns the **embedded** isolate, with the default pool width.
    #[must_use]
    pub fn new(rm: RmMode) -> Self {
        HostIsolateFactory {
            image: embedded_image(),
            pool: DEFAULT_POOL_WORKERS,
            rm,
            park: crate::loopback::ParkVerb::Nothing,
            spawned: std::sync::Mutex::new(Vec::new()),
            // ⊘ OFF unless the VMM says otherwise. A factory that defaulted to granting
            // guest RAM would be granting it on every deployment that never asked, and the
            // grant is the whole boundary.
            guest_ram: None,
        }
    }

    /// ★★★ Grant every isolate this factory spawns a view of guest RAM.
    ///
    /// `fd` must be the VMM's shared, fd-backed guest-memory block
    /// (`memory-backend-memfd,share=on`) and `bytes` its extent. Both come from the VMM;
    /// neither is discoverable by the child. Without this, every
    /// [`kayfabe_isolate::RmBackend::map_guest_ram`] on an isolate from this factory is
    /// [`kayfabe_isolate::RmError::GuestRamUnavailable`] — loudly, rather than degrading
    /// into a copy that cannot work (see that variant).
    #[must_use]
    pub fn with_guest_ram(mut self, fd: OwnedFd, bytes: u64) -> Self {
        self.guest_ram = Some((std::sync::Arc::new(fd), bytes));
        self
    }

    /// Every id this factory was asked for, in order (the birth witness).
    ///
    /// # Panics
    /// If the witness mutex was poisoned by a panic inside a `spawn`.
    #[must_use]
    pub fn spawned(&self) -> Vec<IsolateId> {
        self.spawned.lock().expect("the spawn witness").clone()
    }

    /// ★ Make one verb park **forever** in the child (loopback only).
    ///
    /// The hazard this whole design exists to survive is a verb that does not come back, and
    /// a hazard nothing can produce is a hazard nothing tests. See
    /// [`crate::loopback::ParkVerb`] — the park is a real blocking `read(2)`, so a real
    /// break signal really interrupts it.
    #[must_use]
    pub fn with_park(mut self, park: crate::loopback::ParkVerb) -> Self {
        self.park = park;
        self
    }

    /// Override the pool width (the saturation/backpressure knob).
    ///
    /// # Panics
    /// If `pool` is zero — an isolate that can never issue a verb is a configuration error.
    #[must_use]
    pub fn with_pool_size(mut self, pool: usize) -> Self {
        assert!(pool > 0, "an isolate pool needs at least one worker");
        self.pool = pool;
        self
    }

    /// ★★★ Is an isolate embedded in this build at all?
    ///
    /// The one question a caller may ask about the image, and it has a `bool` answer rather
    /// than a path — there is nothing to name. `false` means `build.rs` ran with
    /// `KAYFABE_ISOLATE_IMAGE_STUB=1`, which only the cross-*check* job may do.
    #[must_use]
    pub fn is_embedded() -> bool {
        embedded_image().is_ok()
    }

    /// Why the embedded image is unusable, if it is.
    ///
    /// # Errors
    /// The publication failure, verbatim.
    pub fn embedded(&self) -> Result<(), String> {
        self.image.as_ref().map(|_| ()).map_err(Clone::clone)
    }

    /// ★ Spawn, keeping the **concrete** type.
    ///
    /// [`IsolateFactory::spawn`] hands back a `Box<dyn Isolate>`, which is right for the core
    /// — it has no business knowing an isolate is a process. It is wrong for a *test of the
    /// containment*, which has to observe the child process itself: its pid, and therefore
    /// its namespaces. Without this the only assertions available about
    /// [`ChildSpec::in_new_namespaces`] would be against a `/bin/sh` fixture, and removing
    /// that one line from `build_isolate` would turn nothing red.
    pub fn spawn_host(&self, id: IsolateId) -> HostIsolate {
        self.spawned.lock().expect("the spawn witness").push(id);
        let built = self.image.clone().and_then(|image| {
            build_isolate(
                image,
                self.rm,
                self.park,
                id,
                self.pool,
                self.guest_ram.as_ref(),
            )
        });
        built.unwrap_or_else(|why| HostIsolate::stillborn(id, self.pool, why))
    }
}

/// Everything that can go wrong before an isolate is usable, as a message.
fn build_isolate(
    image: &ProgramImage,
    rm: RmMode,
    park: crate::loopback::ParkVerb,
    id: IsolateId,
    pool: usize,
    guest_ram: Option<&(std::sync::Arc<OwnedFd>, u64)>,
) -> Result<HostIsolate, String> {
    let (control_ours, control_theirs) =
        UnixDatagram::pair().map_err(|e| format!("control socketpair: {e}"))?;
    let mut ours = Vec::with_capacity(pool);
    let mut spec = ChildSpec::from_image(image, "kayfabe-isolate")
        .map_err(|e| format!("publishing the isolate image for this spawn: {e}"))?
        .arg("--proc")
        .arg(id.proc().to_string())
        .arg("--gpu")
        .arg(id.gpu().0.to_string())
        .arg("--workers")
        .arg(pool.to_string())
        .arg("--rm")
        .arg(rm.as_arg())
        .arg("--park")
        .arg(park.as_arg())
        // ★★★ Born namespaced. The user, pid, network, IPC, UTS and mount namespaces are
        // taken by the `clone` that CREATES this process, not by anything it does afterwards
        // — which is the only way `CLONE_NEWPID` can be had at all, and which means the
        // image cannot decline the namespaces it starts in.
        //
        // ★ There is no opt-out and no fallback to a plain `fork`. A host that will not grant
        // them makes every isolate stillborn with `clone`/`EPERM` in its message. That is the
        // same posture `sandbox::enter` already takes and for the same reason: a silent
        // degrade is how a boundary becomes a comment.
        .in_new_namespaces()
        // ★ And NO device-directory grant. `RESERVED_NEVER_GRANTED_FD` stays empty; the
        // child builds its own sandbox and mints the descriptor inside it.
        .grant(FdGrant::new(OwnedFd::from(control_theirs), CONTROL_FD));
    // ★ The park witness, granted ONLY when a park is armed. `ParkVerb::Nothing` is every
    // production isolate, and it gets no pipe, no grant and no write — the descriptor number
    // is reserved rather than reused so the layout does not depend on a test knob.
    let park_witness = if park == crate::loopback::ParkVerb::Nothing {
        None
    } else {
        let (reader, writer) = std::io::pipe().map_err(|e| format!("park witness pipe: {e}"))?;
        let ours = writer
            .try_clone()
            .map_err(|e| format!("keeping our own park deadline writer: {e}"))?;
        spec = spec.grant(FdGrant::new(OwnedFd::from(writer), PARK_WITNESS_FD));
        Some((reader, ours))
    };
    let (park_witness, park_deadline) = match park_witness {
        Some((r, w)) => (Some(r), Some(w)),
        None => (None, None),
    };
    // ★★★ Guest RAM, granted at a FIXED number and only when the VMM has some. Each spawn
    // takes its own `dup`, so one isolate's exit cannot close the next one's view — the
    // same rule the program image already follows.
    if let Some((fd, bytes)) = guest_ram {
        let dup = fd
            .try_clone()
            .map_err(|e| format!("duplicating the guest-RAM descriptor for this spawn: {e}"))?;
        spec = spec
            .grant(FdGrant::new(dup, GUEST_RAM_FD))
            .arg("--guest-ram-bytes")
            .arg(bytes.to_string());
    }
    for i in 0..pool {
        let (mine, theirs) = UnixStream::pair().map_err(|e| format!("worker socketpair: {e}"))?;
        ours.push(mine);
        spec = spec.grant(FdGrant::new(
            OwnedFd::from(theirs),
            WORKER_FD_BASE + i as i32,
        ));
    }

    let child =
        SandboxChild::spawn(spec).map_err(|e| format!("spawning the embedded isolate: {e}"))?;

    // ★ A synchronous readiness handshake, one frame per worker. Without it the first verb
    // of the first guest operation is where "the driver is not loaded" surfaces — miles
    // from the cause, in a path that has to classify it as a host failure. `spawn` blocks
    // here, which is legal (R1 is asserted, no lock is held) and is what makes a bring-up
    // failure a *startup* diagnosis.
    let control = Arc::new(control_ours);
    // ★ One registry per isolate, shared by its pool. Minted here rather than per worker
    // so that a token means the same thing whichever slot served the request.
    let exports = Arc::new(ExportRegistry::new());
    let mut cancels = Vec::with_capacity(pool);
    let mut slots = Vec::with_capacity(pool);
    let mut buf = Vec::new();
    for (i, sock) in ours.into_iter().enumerate() {
        let mut r = &sock;
        match read_frame(&mut r, &mut buf) {
            Ok(true) => match Reply::decode(&buf) {
                Ok(Reply::Unit) => {}
                Ok(Reply::Failed(e)) => {
                    return Err(format!("isolate refused to start: {e:?}"));
                }
                Ok(other) => return Err(format!("isolate sent {other:?} as its hello")),
                Err(e) => return Err(format!("isolate's hello did not decode: {e}")),
            },
            Ok(false) => return Err("isolate exited before its hello".to_owned()),
            Err(e) => return Err(format!("reading the isolate's hello: {e}")),
        }
        let sock = Arc::new(sock);
        let sink = Arc::new(HostCancelSink {
            worker: WorkerId(i as u32),
            reply: Arc::clone(&sock),
            control: Arc::clone(&control),
            state: Mutex::new(CancelState::default()),
        });
        cancels.push(Arc::clone(&sink));
        slots.push(Slot::Idle(Box::new(Worker::with_cancel(
            id,
            WorkerId(i as u32),
            Box::new(ProxyRmBackend {
                isolate: id,
                sock,
                cancel: Arc::clone(&sink),
                buf: Vec::new(),
                exports: Arc::clone(&exports),
            }),
            sink as Arc<dyn CancelSink>,
        ))));
    }

    Ok(HostIsolate {
        id,
        child: Some(child),
        slots,
        cancels,
        next_txn: 0,
        retired: false,
        spawn_error: None,
        exports,
        park_witness,
        park_deadline,
    })
}

impl IsolateFactory for HostIsolateFactory {
    fn spawn(&self, id: IsolateId) -> Box<dyn Isolate> {
        Box::new(self.spawn_host(id))
    }
}

// Compile-time proof that the real implementations satisfy the same thread contract the
// core stores the mocks under (decision #17).
kayfabe_util::assert_send_sync!(
    HostIsolate,
    HostIsolateFactory,
    ProxyRmBackend,
    HostCancelSink
);

#[cfg(test)]
mod tests {
    use super::*;
    use kayfabe_arch::ids::GpuId;

    #[test]
    fn an_unknown_rm_mode_is_refused_rather_than_defaulted_to_real() {
        assert_eq!(RmMode::parse("real"), Some(RmMode::Real));
        assert_eq!(RmMode::parse("loopback"), Some(RmMode::Loopback));
        assert_eq!(RmMode::parse(""), None);
        assert_eq!(RmMode::parse("Real"), None);
        assert_eq!(RmMode::parse("fake"), None);
    }

    #[test]
    fn control_messages_round_trip_and_malformed_ones_are_refused() {
        for (w, txn, r) in [
            (0u32, 1u64, CancelReason::ProcExit),
            (3, 9, CancelReason::Watchdog),
        ] {
            let mut msg = [0u8; CONTROL_MSG_LEN];
            msg[..4].copy_from_slice(&w.to_le_bytes());
            msg[4..12].copy_from_slice(&txn.to_le_bytes());
            msg[12] = reason_code(r);
            assert_eq!(decode_control(&msg), Some((w, txn, reason_code(r))));
        }
        assert_eq!(decode_control(&[0u8; CONTROL_MSG_LEN - 1]), None, "short");
        assert_eq!(decode_control(&[0u8; CONTROL_MSG_LEN + 1]), None, "long");
        let mut bad_reason = [0u8; CONTROL_MSG_LEN];
        bad_reason[12] = 9;
        assert_eq!(decode_control(&bad_reason), None, "unknown reason");
        assert_eq!(decode_control(&[0u8; CONTROL_MSG_LEN]), None, "reason 0");
    }

    #[test]
    fn every_cancel_reason_has_a_distinct_nonzero_code() {
        let codes: Vec<u8> = [
            CancelReason::ProcExit,
            CancelReason::DeviceReset,
            CancelReason::Watchdog,
            CancelReason::GuestSignal,
        ]
        .into_iter()
        .map(reason_code)
        .collect();
        assert!(!codes.contains(&0), "zero must not name a reason");
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "codes collide: {codes:?}");
    }

    /// A spawn that cannot work produces a STILLBORN isolate rather than a panic or a
    /// half-built one — the shape [`HostIsolate::spawn_error`] documents, exercised.
    ///
    /// ★ The fixture is a factory whose **image** is unusable, because that is now the only
    /// way a spawn can fail before it starts: there is no path to point at a missing file.
    /// The struct is built here by hand rather than through a constructor, deliberately —
    /// a public "factory with a broken image" constructor would be a door into spawning
    /// something other than the embedded isolate, which is the whole thing this change
    /// removes.
    #[test]
    fn an_unusable_image_yields_a_retired_isolate_that_names_why() {
        let f = HostIsolateFactory {
            image: Err("no image in this build".to_owned()),
            pool: 4,
            rm: RmMode::Loopback,
            park: crate::loopback::ParkVerb::Nothing,
            spawned: std::sync::Mutex::new(Vec::new()),
            guest_ram: None,
        };
        assert_eq!(f.embedded(), Err("no image in this build".to_owned()));
        let id = IsolateId::new(1, GpuId(0));
        let mut iso = f.spawn(id);
        assert_eq!(iso.id(), id);
        assert!(iso.is_retired(), "a stillborn isolate refuses checkouts");
        assert!(iso.checkout().is_none());
        assert_eq!(iso.in_flight(), 0);
        assert!(iso.is_quiesced(), "and it is safe to reap immediately");
        assert_eq!(f.spawned(), vec![id]);
    }

    /// The stillborn isolate carries the reason, on the concrete type that has it.
    #[test]
    fn a_stillborn_isolate_names_why_it_never_started() {
        let iso = HostIsolate::stillborn(
            IsolateId::new(2, GpuId(0)),
            2,
            "no image in this build".to_owned(),
        );
        assert_eq!(iso.spawn_error(), Some("no image in this build"));
    }

    /// ★★★ **E1 — bench gap 7, reproduced and then closed, in one test.**
    ///
    /// `bench_rebuild_notes.md` §5 row 7: *"a **failed** real isolate is indistinguishable
    /// from the stillborn one at the seam"*. The first half below **reproduces** that —
    /// every observable the core had before E1 agrees between a host spawn that failed and
    /// a build that deliberately has no forwarding plane — so the second half is measuring
    /// a real ambiguity rather than asserting into a vacuum. Delete
    /// [`Isolate::refusal`] and the first half still passes; that is the point of writing
    /// it out.
    ///
    /// ⊘ Driven through the two REAL implementors. `MockIsolate::refusal` is `None` by
    /// construction, deliberately: a mock that answered here would be the thing under test
    /// observing itself.
    #[test]
    fn a_failed_host_isolate_is_distinguishable_from_a_deliberately_planeless_one() {
        use kayfabe_isolate::{IsolateCensus, RefusalKind, StillbornIsolates};

        const NO_PLANE_WHY: &str = "this build has no forwarding plane: the object model \
                                    accepts protocol facts and no host verb can be issued";
        const FAILED_WHY: &str = "spawning the embedded isolate: EPERM (clone)";

        let mut failed = HostIsolate::stillborn(IsolateId::new(2, GpuId(0)), 4, FAILED_WHY.into());
        let sf = StillbornIsolates::new(NO_PLANE_WHY);
        let mut planeless = sf.spawn(IsolateId::new(3, GpuId(0)));

        // ---- gap 7, reproduced: every PRE-E1 observable agrees -----------------------
        assert_eq!(failed.is_retired(), planeless.is_retired());
        assert!(failed.is_retired(), "and both really are refusing");
        assert!(failed.checkout().is_none() && planeless.checkout().is_none());
        assert_eq!(failed.idle_workers(), planeless.idle_workers());
        assert_eq!(failed.in_flight(), planeless.in_flight());
        assert_eq!(failed.checked_out(), planeless.checked_out());
        assert!(failed.is_quiesced() && planeless.is_quiesced());

        // ---- and the E1 seam separates them ------------------------------------------
        let f = failed.refusal().expect("a failed spawn refuses");
        let p = planeless.refusal().expect("a plane-less build refuses");
        assert_eq!(f.kind, RefusalKind::SpawnFailed);
        assert_eq!(p.kind, RefusalKind::NoPlane);
        assert_ne!(f.kind, p.kind, "the KIND is what a check may branch on");
        assert_ne!(f.why, p.why, "and the sentences differ too");
        assert_eq!(f.why, FAILED_WHY);
        assert_eq!(p.why, NO_PLANE_WHY);

        // ---- the census counts them apart, and prefers the actionable one -------------
        let mut c = IsolateCensus::default();
        c.observe(&*planeless);
        c.observe(&failed);
        assert_eq!((c.live, c.no_plane, c.spawn_failed), (2, 1, 1));
        assert_eq!(c.refusing(), 2);
        assert_eq!(
            c.first,
            Some((RefusalKind::SpawnFailed, FAILED_WHY.to_owned())),
            "the one line a report has room for must be the one that means the host is wrong"
        );
    }

    /// ⊘ A **live** isolate reports no refusal, and still reports none after an ordinary
    /// [`Isolate::retire`] — the control for the test above. Without it, `refusal()`
    /// returning `Some` unconditionally would pass everything else.
    ///
    /// Driven against a real loopback child: it is the only arm that can be spawned on a
    /// runner with no GPU.
    #[test]
    fn a_live_isolate_reports_no_refusal_even_after_retire() {
        let f = HostIsolateFactory::new(RmMode::Loopback);
        let mut iso = f.spawn_host(IsolateId::new(9, GpuId(0)));
        assert_eq!(
            iso.spawn_error(),
            None,
            "the fixture must really have come up"
        );
        assert!(iso.refusal().is_none(), "a working isolate refuses nothing");
        iso.retire();
        assert!(
            iso.refusal().is_none(),
            "an ordinary teardown is not a refusal to investigate"
        );
    }

    /// ★★ The image really is embedded, and it really is a **static** ELF.
    ///
    /// Asserted on the bytes rather than on the build script's exit status: a build that
    /// took the `KAYFABE_ISOLATE_IMAGE_STUB=1` opt-out succeeds and embeds nothing, and this
    /// is the assertion that makes such a build unable to pass a test run.
    #[test]
    fn the_embedded_image_is_a_static_elf() {
        let bytes = embedded_isolate_bytes();
        assert!(
            bytes.len() > 4096,
            "no isolate image is embedded ({} bytes) — a build with \
             KAYFABE_ISOLATE_IMAGE_STUB=1 cannot pass a test run",
            bytes.len()
        );
        assert_eq!(&bytes[..4], b"\x7fELF", "the image must be an ELF");
        assert!(
            HostIsolateFactory::is_embedded(),
            "and it must publish into a memfd"
        );
        // ★ Static: no `PT_INTERP`. A dynamic isolate cannot be `exec`'d from inside its own
        // `pivot_root`ed sandbox, because the loader it needs is not in the sandbox — which
        // is exactly why the image is built for musl. Checked by walking the program headers
        // rather than by shelling out to `file`, so the assertion holds with no tools
        // installed.
        assert!(
            !has_interpreter(bytes),
            "the embedded isolate is dynamically linked (it has a PT_INTERP); it must be static"
        );
    }

    /// Does this 64-bit little-endian ELF carry a `PT_INTERP` program header?
    fn has_interpreter(bytes: &[u8]) -> bool {
        const PT_INTERP: u32 = 3;
        let read_u16 = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        let read_u64 = |o: usize| {
            u64::from_le_bytes(
                bytes[o..o + 8]
                    .try_into()
                    .expect("eight bytes are eight bytes"),
            )
        };
        assert_eq!(bytes[4], 2, "the image must be ELF64");
        assert_eq!(bytes[5], 1, "the image must be little-endian");
        let phoff = usize::try_from(read_u64(0x20)).expect("a sane program-header offset");
        let phentsize = read_u16(0x36) as usize;
        let phnum = read_u16(0x38) as usize;
        (0..phnum).any(|i| {
            let at = phoff + i * phentsize;
            u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes")) == PT_INTERP
        })
    }

    #[test]
    #[should_panic(expected = "at least one worker")]
    fn a_zero_width_pool_is_refused() {
        let _ = HostIsolateFactory::new(RmMode::Loopback).with_pool_size(0);
    }
}
