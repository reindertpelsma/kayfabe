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

use crate::proto::{Envelope, Reply, Request, WireError, engine_code, read_frame, write_frame};
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuVa};
use kayfabe_isolate::{
    CancelHandle, CancelReason, CancelSink, CeExecutor, CeSource, CeSubCopy, DEFAULT_POOL_WORKERS,
    HostHandle, Isolate, IsolateFactory, IsolateId, RmBackend, RmError, Txn, Worker, WorkerId,
};
use kayfabe_linux_raw::{ChildSpec, FdGrant, SandboxChild};
use kayfabe_vmm::SurfaceHandle;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// fd number of the control datagram socket in the child.
pub const CONTROL_FD: i32 = 3;
/// ★ The number the C parks `NVKVM_DEV_DIRFD` on, **reserved and never granted** here. See
/// the module docs: a parent-opened `/dev` descriptor is an escape, and the vacancy is the
/// fix rather than an oversight.
pub const RESERVED_NEVER_GRANTED_FD: i32 = 4;
/// fd number of pool worker 0's stream in the child; worker *n* is `WORKER_FD_BASE + n`.
pub const WORKER_FD_BASE: i32 = 5;

// ★ The descriptor contract, checked at COMPILE time rather than by a test. It is a
// relationship between three constants, so a runtime assertion could only ever be
// constant-folded — and a "test" a compiler can prove is not a test. A future edit that
// collides two grants fails the build.
const _: () = {
    assert!(CONTROL_FD >= 3, "grants start above the standard streams");
    assert!(CONTROL_FD != RESERVED_NEVER_GRANTED_FD);
    assert!(
        WORKER_FD_BASE > RESERVED_NEVER_GRANTED_FD,
        "workers come after the reserved hole"
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

    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        let reply = self.call(Request::ExportSurface {
            memory: memory.raw(),
        })?;
        match self.lift(reply)? {
            Reply::Surface(s) => Ok(SurfaceHandle(s)),
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
}

impl HostIsolate {
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
        }
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
    program: PathBuf,
    pool: usize,
    rm: RmMode,
    park: crate::loopback::ParkVerb,
    /// Every id this factory was asked for, in order — the isolate-per-`(Proc, GpuId)`
    /// witness, matching `MockIsolateFactory::spawned` so a test can assert the same
    /// property against either.
    pub spawned: Vec<IsolateId>,
}

impl HostIsolateFactory {
    /// A factory that runs `program` as its isolate, with the default pool width.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, rm: RmMode) -> Self {
        HostIsolateFactory {
            program: program.into(),
            pool: DEFAULT_POOL_WORKERS,
            rm,
            park: crate::loopback::ParkVerb::Nothing,
            spawned: Vec::new(),
        }
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

    /// ★ Locate the isolate binary beside the current executable.
    ///
    /// `KAYFABE_ISOLATE_BIN` overrides it. Otherwise the search is *"a sibling of this
    /// program"*, which is where both a cargo build (`target/<profile>/`) and an install
    /// tree put it — and, critically, it is not `PATH`: an isolate found on `PATH` is an
    /// isolate an environment variable chose, and this process hands that binary a
    /// descriptor for `/dev`.
    ///
    /// # Errors
    /// A message naming what was looked for and where. Deliberately a `String`: the caller
    /// is a composition root that is about to refuse loudly, not a retry loop.
    pub fn locate_program() -> Result<PathBuf, String> {
        if let Some(explicit) = std::env::var_os("KAYFABE_ISOLATE_BIN") {
            let p = PathBuf::from(explicit);
            return if p.is_file() {
                Ok(p)
            } else {
                Err(format!(
                    "KAYFABE_ISOLATE_BIN names {p:?}, which is not a file"
                ))
            };
        }
        let me = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let dir = me
            .parent()
            .ok_or_else(|| format!("{me:?} has no parent directory"))?;
        // A cargo integration test lives in `target/<profile>/deps/`, one level below the
        // binaries. Both are checked, nearest first, and nothing else is.
        for cand in [dir.join("kayfabe-isolate"), sibling_of_deps(dir)] {
            if cand.is_file() {
                return Ok(cand);
            }
        }
        Err(format!(
            "no `kayfabe-isolate` beside {me:?} (set KAYFABE_ISOLATE_BIN)"
        ))
    }
}

fn sibling_of_deps(dir: &Path) -> PathBuf {
    dir.parent().unwrap_or(dir).join("kayfabe-isolate")
}

/// Everything that can go wrong before an isolate is usable, as a message.
fn build_isolate(
    program: &Path,
    rm: RmMode,
    park: crate::loopback::ParkVerb,
    id: IsolateId,
    pool: usize,
) -> Result<HostIsolate, String> {
    let (control_ours, control_theirs) =
        UnixDatagram::pair().map_err(|e| format!("control socketpair: {e}"))?;
    let mut ours = Vec::with_capacity(pool);
    let mut spec = ChildSpec::new(program)
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
        // ★ And NO device-directory grant. `RESERVED_NEVER_GRANTED_FD` stays empty; the
        // child builds its own sandbox and mints the descriptor inside it.
        .grant(FdGrant::new(OwnedFd::from(control_theirs), CONTROL_FD));
    for i in 0..pool {
        let (mine, theirs) = UnixStream::pair().map_err(|e| format!("worker socketpair: {e}"))?;
        ours.push(mine);
        spec = spec.grant(FdGrant::new(
            OwnedFd::from(theirs),
            WORKER_FD_BASE + i as i32,
        ));
    }

    let child = SandboxChild::spawn(spec).map_err(|e| format!("spawning {program:?}: {e}"))?;

    // ★ A synchronous readiness handshake, one frame per worker. Without it the first verb
    // of the first guest operation is where "the driver is not loaded" surfaces — miles
    // from the cause, in a path that has to classify it as a host failure. `spawn` blocks
    // here, which is legal (R1 is asserted, no lock is held) and is what makes a bring-up
    // failure a *startup* diagnosis.
    let control = Arc::new(control_ours);
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
    })
}

impl IsolateFactory for HostIsolateFactory {
    fn spawn(&mut self, id: IsolateId) -> Box<dyn Isolate> {
        self.spawned.push(id);
        match build_isolate(&self.program, self.rm, self.park, id, self.pool) {
            Ok(iso) => Box::new(iso),
            Err(why) => Box::new(HostIsolate::stillborn(id, self.pool, why)),
        }
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
    #[test]
    fn a_missing_isolate_binary_yields_a_retired_isolate_that_names_why() {
        let mut f = HostIsolateFactory::new("/kayfabe/no/such/isolate", RmMode::Loopback);
        let id = IsolateId::new(1, GpuId(0));
        let mut iso = f.spawn(id);
        assert_eq!(iso.id(), id);
        assert!(iso.is_retired(), "a stillborn isolate refuses checkouts");
        assert!(iso.checkout().is_none());
        assert_eq!(iso.in_flight(), 0);
        assert!(iso.is_quiesced(), "and it is safe to reap immediately");
        assert_eq!(f.spawned, vec![id]);
    }

    #[test]
    #[should_panic(expected = "at least one worker")]
    fn a_zero_width_pool_is_refused() {
        let _ = HostIsolateFactory::new("/bin/true", RmMode::Loopback).with_pool_size(0);
    }
}
