//! The **child** side: what runs inside the isolate process.
//!
//! One thread per pool worker, each owning its own request/reply stream, plus one control
//! thread that does nothing but turn a cancel datagram into a `tgkill` at the right thread.
//!
//! ## ★ Why the control thread cannot be the worker loop
//!
//! The C dispatches its interrupt **on the reader thread**, inline
//! (`C: src/stub/nvkvm_stub.c:2467-2469`), and its own analysis names the cost: while that
//! thread is inside a long handler, interrupts stall. Here the cancel path is a dedicated
//! thread blocked in `recv(2)` on a socket nothing else uses, so a cancel is serviced while
//! every worker is busy — which is the only state in which a cancel is ever wanted.
//!
//! ## ★★ The txn check, and the bug it prevents
//!
//! A worker publishes its txn immediately before the verb and clears it immediately after
//! (`C: src/stub/nvkvm_stub.c:1277-1281`, the same two lines). The control thread signals
//! only if the slot is *still* on the txn the request named. Without it a cancel races the
//! completion and lands on **an unrelated later operation** — the sharpest bug in this
//! area, because the damage is done to an innocent op (`kayfabe_isolate::Txn`'s own docs).
//!
//! ## What a protocol violation does
//!
//! It kills the worker's loop, which the parent sees as a closed channel and reports as
//! [`kayfabe_isolate::RmError::Wedged`]. There is no "recover and resynchronise": the peer
//! is the parent, a parent we cannot parse is a parent we cannot serve, and a channel that
//! attempts to resynchronise after a framing error is exactly the desynchronisation hazard
//! §7.2 forbids, reached from the other end.

use crate::isolate::{CONTROL_FD, RmMode, WORKER_FD_BASE, decode_control};
use crate::loopback::{LoopbackRm, LoopbackShared, ParkVerb};
use crate::proto::{
    Envelope, Reply, Request, WireError, engine_from_code, read_frame, write_frame,
};
use crate::rm::{HostRmBackend, RmConnection};
use kayfabe_arch::ids::{ClassId, ControlCmd, GpuId, GpuVa};
use kayfabe_isolate::{CeExecutor, CeSource, CeSubCopy, HostHandle, IsolateId, RmBackend, RmError};
use kayfabe_linux_raw::sandbox::{self, SandboxPolicy};
use kayfabe_linux_raw::{
    ThreadId, adopt_inherited_fd, current_thread_id, install_break_handler, interrupt_thread,
};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::sync::{Arc, Mutex};

/// How the child was invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildArgs {
    /// The owning proc's id.
    pub proc: u32,
    /// The GPU this isolate is the sandbox for.
    pub gpu: u32,
    /// Pool width — one worker thread and one socket each.
    pub workers: usize,
    /// Which RM to speak to.
    pub rm: RmMode,
    /// Which verb parks forever (loopback only).
    pub park: ParkVerb,
}

impl ChildArgs {
    /// Parse `--proc N --gpu N --workers N --rm MODE [--park VERB]`.
    ///
    /// Hand-rolled rather than a parser crate, and strict: an unknown flag, a missing
    /// value, or an unparseable number is a refusal. This process is handed a descriptor
    /// for `/dev`; it does not get to guess what it was asked for.
    ///
    /// # Errors
    /// A message naming the offending argument.
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let mut proc = None;
        let mut gpu = None;
        let mut workers = None;
        let mut rm = None;
        let mut park = ParkVerb::Nothing;
        let mut it = args.into_iter();
        while let Some(flag) = it.next() {
            let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--proc" => proc = Some(value.parse().map_err(|_| format!("--proc {value}"))?),
                "--gpu" => gpu = Some(value.parse().map_err(|_| format!("--gpu {value}"))?),
                "--workers" => {
                    workers = Some(value.parse().map_err(|_| format!("--workers {value}"))?);
                }
                "--rm" => rm = Some(RmMode::parse(&value).ok_or(format!("--rm {value}"))?),
                "--park" => park = ParkVerb::parse(&value).ok_or(format!("--park {value}"))?,
                other => return Err(format!("unknown flag {other}")),
            }
        }
        let workers = workers.ok_or("--workers is required")?;
        if workers == 0 {
            return Err("--workers 0 is an isolate that can never issue a verb".to_owned());
        }
        Ok(ChildArgs {
            proc: proc.ok_or("--proc is required")?,
            gpu: gpu.ok_or("--gpu is required")?,
            workers,
            rm: rm.ok_or("--rm is required")?,
            park,
        })
    }

    /// The isolate identity this child is the sandbox for.
    #[must_use]
    pub fn isolate(&self) -> IsolateId {
        IsolateId::new(self.proc, GpuId(self.gpu))
    }
}

/// One worker slot's cancellation-relevant state, as the control thread sees it.
#[derive(Debug, Default, Clone, Copy)]
struct SlotState {
    thread: Option<ThreadId>,
    /// The txn currently on the wire, or 0 for "between verbs".
    txn: u64,
}

/// Run the isolate. Returns the process exit code.
///
/// Never panics on a peer's behalf: every failure the parent could have caused becomes a
/// closed channel or a reported error, because a child that aborts on malformed input is a
/// child a guest can crash.
#[must_use]
pub fn serve(args: &ChildArgs) -> i32 {
    if let Err(e) = install_break_handler() {
        eprintln!("kayfabe-isolate: cannot install the break handler: {e}");
        return 2;
    }
    let id = args.isolate();

    let mut sockets = Vec::with_capacity(args.workers);
    for i in 0..args.workers {
        match adopt_inherited_fd(WORKER_FD_BASE + i as i32) {
            Ok(fd) => sockets.push(UnixStream::from(fd)),
            Err(e) => {
                eprintln!("kayfabe-isolate: worker {i}'s descriptor was not granted: {e}");
                return 2;
            }
        }
    }
    let control = match adopt_inherited_fd(CONTROL_FD) {
        Ok(fd) => UnixDatagram::from(fd),
        Err(e) => {
            eprintln!("kayfabe-isolate: the control descriptor was not granted: {e}");
            return 2;
        }
    };

    // ★ Build the backends BEFORE announcing readiness, so a bring-up failure is reported
    // on the hello frame rather than on the first guest operation.
    let backends = match build_backends(args, id) {
        Ok(b) => b,
        Err(why) => {
            eprintln!("kayfabe-isolate: {why}");
            // Every worker gets the refusal, so the parent's handshake fails on the first
            // socket it reads rather than blocking on a channel nobody will ever write.
            for mut sock in sockets {
                let _ = write_frame(
                    &mut sock,
                    &Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)).encode(),
                );
            }
            return 3;
        }
    };

    let slots: Arc<Vec<Mutex<SlotState>>> = Arc::new(
        (0..args.workers)
            .map(|_| Mutex::new(SlotState::default()))
            .collect(),
    );

    let control_slots = Arc::clone(&slots);
    let control_thread = std::thread::spawn(move || control_loop(&control, &control_slots));

    let mut threads = Vec::with_capacity(args.workers);
    for (i, (sock, backend)) in sockets.into_iter().zip(backends).enumerate() {
        let slots = Arc::clone(&slots);
        threads.push(std::thread::spawn(move || {
            worker_loop(i, sock, backend, &slots);
        }));
    }
    for t in threads {
        let _ = t.join();
    }
    // The control thread is blocked in `recv` on a socket whose only writer is the parent.
    // It ends when the parent's end closes, which has already happened if every worker
    // loop returned — but never join it unconditionally: §7.5's rule is that we do not
    // block on a thread that may be inside an unbounded wait.
    drop(control_thread);
    0
}

fn build_backends(args: &ChildArgs, id: IsolateId) -> Result<Vec<Box<dyn RmBackend>>, String> {
    match args.rm {
        RmMode::Real => {
            // ★★★ The containment and the capability are created by ONE call, in that
            // order, and there is no other way to obtain the second. The parent used to
            // hand a `/dev` descriptor down on fd 4 and that descriptor could name
            // `../etc/shadow`; see `kayfabe_linux_raw::sandbox` and
            // `crate::isolate`'s "fd 4 is vacant" note.
            //
            // ★ Still single-threaded here, deliberately: the rootless arm of
            // `sandbox::enter` needs `CLONE_NEWUSER`, which the kernel refuses to a
            // multi-threaded process. Worker threads start after `build_backends` returns.
            //
            // Fail CLOSED. A sandbox that could not be built is not a warning, it is the
            // end of this isolate: the error propagates to the hello frame and the parent
            // reports a startup failure.
            let dev = sandbox::enter(&SandboxPolicy::for_gpu(args.gpu))
                .map_err(|e| format!("the isolate sandbox could not be built: {e}"))?;
            let conn =
                Arc::new(RmConnection::open(&dev, GpuId(args.gpu)).map_err(|e| e.to_string())?);
            Ok((0..args.workers)
                .map(|_| Box::new(HostRmBackend::new(id, Arc::clone(&conn))) as Box<dyn RmBackend>)
                .collect())
        }
        RmMode::Loopback => {
            let shared = LoopbackShared::new(args.park).map_err(|e| format!("park pipe: {e}"))?;
            let mut out: Vec<Box<dyn RmBackend>> = Vec::with_capacity(args.workers);
            for _ in 0..args.workers {
                out.push(Box::new(
                    LoopbackRm::new(id, Arc::clone(&shared))
                        .map_err(|e| format!("park pipe dup: {e}"))?,
                ));
            }
            Ok(out)
        }
    }
}

fn control_loop(control: &UnixDatagram, slots: &[Mutex<SlotState>]) {
    let mut buf = [0u8; 64];
    loop {
        let n = match control.recv(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            // A cancel can land on this thread too, between datagrams. It means nothing
            // here; the loop simply continues.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        let Some((worker, txn, _reason)) = decode_control(&buf[..n]) else {
            // A malformed control message is DROPPED, never guessed at. A mis-decoded
            // cancel lands on an innocent operation.
            continue;
        };
        let Some(slot) = slots.get(worker as usize) else {
            continue;
        };
        // Read the target out and release the lock BEFORE the syscall: signalling under a
        // lock a worker also takes is how a cancel path deadlocks the thing it is
        // cancelling.
        let target = {
            let s = slot.lock().unwrap_or_else(|e| e.into_inner());
            (s.txn == txn).then_some(s.thread).flatten()
        };
        if let Some(thread) = target {
            let _ = interrupt_thread(thread);
        }
    }
}

fn worker_loop(
    index: usize,
    sock: UnixStream,
    mut backend: Box<dyn RmBackend>,
    slots: &[Mutex<SlotState>],
) {
    {
        let mut s = slots[index].lock().unwrap_or_else(|e| e.into_inner());
        s.thread = Some(current_thread_id());
    }
    let mut sock = sock;
    // The hello: this worker is ready. Written by the worker itself rather than by the
    // spawning code, so "ready" means "this thread is in its loop", not "a thread was
    // created".
    if write_frame(&mut sock, &Reply::Unit.encode()).is_err() {
        return;
    }

    let mut buf = Vec::new();
    loop {
        match read_frame(&mut sock, &mut buf) {
            Ok(true) => {}
            // The parent closed, or abandoned this channel (§7.5). Either way there is
            // nothing left to serve.
            Ok(false) => return,
            // A cancel that arrived while this worker was BETWEEN verbs. There is nothing
            // to interrupt, so it is spent here rather than left to break the next read.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
        let Ok(envelope) = Envelope::decode(&buf) else {
            return;
        };
        {
            let mut s = slots[index].lock().unwrap_or_else(|e| e.into_inner());
            s.txn = envelope.txn;
        }
        let reply = execute(&mut *backend, envelope.request);
        {
            let mut s = slots[index].lock().unwrap_or_else(|e| e.into_inner());
            s.txn = 0;
        }
        if write_frame(&mut sock, &reply.encode()).is_err() {
            return;
        }
    }
}

/// ★ Largest [`Request::FbRead`] this child will serve, in bytes.
///
/// A page-table page is one 4 KiB table; the bound is generous enough that no legitimate
/// decode meets it and small enough that the reply always fits a frame
/// ([`crate::proto::FRAME_MAX`] is 1 MiB). It exists because `len` arrives **on the wire**
/// and is therefore attacker-influenced in every threat model where the parent can be
/// compromised — `vec![0u8; len]` with an unchecked `len` is a remote allocation.
const FB_READ_MAX: usize = 1 << 16;

/// The status a too-large [`Request::FbRead`] is refused with. Opaque on the wire and
/// named here, so a refusal reads as a refusal and never as an empty page.
const FB_READ_TOO_LARGE: u32 = 0xFB00_0001;

/// Run one verb and shape its answer. The only place the wire vocabulary meets the port's.
fn execute(rm: &mut dyn RmBackend, request: Request) -> Reply {
    fn handle(r: Result<HostHandle, RmError>) -> Reply {
        match r {
            Ok(h) => Reply::Handle(h.raw()),
            Err(e) => failed(e),
        }
    }
    fn unit(r: Result<(), RmError>) -> Reply {
        match r {
            Ok(()) => Reply::Unit,
            Err(e) => failed(e),
        }
    }
    match request {
        Request::Alloc {
            parent,
            class,
            params,
        } => handle(rm.alloc(raw(parent), ClassId(class), &params)),
        Request::AllocVaSpace => handle(rm.alloc_vaspace()),
        Request::AllocSysmem { len } => handle(rm.alloc_sysmem(len)),
        Request::AllocChannel { vas, engine } => match engine_from_code(engine) {
            // An engine code we do not recognise is a refusal, never a default — the GR-1
            // wrong-runlist class.
            None => Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            Some(engine) => match rm.alloc_channel(raw(vas), engine) {
                Ok((h, token)) => Reply::HandleAndToken(h.raw(), token),
                Err(e) => failed(e),
            },
        },
        Request::AllocEngineObject {
            chan,
            class,
            params,
        } => handle(rm.alloc_engine_object(raw(chan), ClassId(class), &params)),
        Request::Schedule { chan } => unit(rm.schedule(raw(chan))),
        Request::Free { obj } => unit(rm.free(raw(obj))),
        Request::Control {
            obj,
            cmd,
            mut payload,
        } => match rm.control(raw(obj), ControlCmd(cmd), &mut payload) {
            Ok(()) => Reply::Payload(payload),
            Err(e) => failed(e),
        },
        Request::MapGpuVa {
            vas,
            memory,
            len,
            at,
        } => match rm.map_gpu_va(raw(vas), raw(memory), len, GpuVa(at)) {
            Ok(va) => Reply::Va(va),
            Err(e) => failed(e),
        },
        Request::UnmapGpuVa { vas, gpu_va } => unit(rm.unmap_gpu_va(raw(vas), gpu_va)),
        // ★★★ #102 stage C3 — a read of the fabricated aperture. `len` is peer-supplied,
        // so it is bounded HERE, before a buffer is allocated: an unbounded `vec![0; len]`
        // driven by a frame is a remote allocation, and the fact that the peer is our own
        // parent is not a reason to write code that trusts a length.
        Request::FbRead { phys, len } => match usize::try_from(len) {
            Ok(n) if n <= FB_READ_MAX => {
                let mut buf = vec![0u8; n];
                match rm.fb_read(phys, &mut buf) {
                    Ok(true) => Reply::FbBytes {
                        covered: true,
                        bytes: buf,
                    },
                    Ok(false) => Reply::FbBytes {
                        covered: false,
                        bytes: Vec::new(),
                    },
                    Err(e) => failed(e),
                }
            }
            _ => failed(RmError::Other(FB_READ_TOO_LARGE)),
        },
        Request::RingDoorbell { token } => unit(rm.ring_doorbell(token)),
        Request::ExportSurface { memory } => match rm.export_surface(raw(memory)) {
            Ok(s) => Reply::Surface(s.0),
            Err(e) => failed(e),
        },
        Request::CeCopy {
            vas,
            dst,
            src,
            len,
            src_is_const,
            by_ours,
        } => unit(rm.ce_copy(
            raw(vas),
            CeSubCopy {
                dst,
                src: if src_is_const == 0 {
                    CeSource::Address(src)
                } else {
                    // A wide `src` carrying a narrow constant: take the low 32 bits and
                    // do not reject the rest. The pattern register IS 32 bits, and a
                    // hostile peer setting high bits is describing nothing — refusing
                    // here would turn a meaningless field into a denial of service.
                    CeSource::Constant(src as u32)
                },
                len,
                by: if by_ours == 0 {
                    CeExecutor::HostCe
                } else {
                    CeExecutor::Ours
                },
            },
        )),
    }
}

/// ★ The child's own view of a handle. It stamps [`IsolateId::NONE`] because **the child
/// has no business asserting a namespace**: the parent stamps every handle it receives with
/// the connection it asked on, so a provenance claim from this side would be a claim the
/// parent overrides anyway — and one that a compromised child could make.
fn raw(value: u64) -> HostHandle {
    if value == 0 {
        HostHandle::NULL
    } else {
        HostHandle::new(kayfabe_isolate::IsolateId::NONE, value)
    }
}

/// Shape an [`RmError`] for the wire. The two variants with no wire form
/// ([`RmError::Wedged`], [`RmError::ForeignHandle`]) are unreachable from a backend, and
/// are mapped to an opaque status rather than silently dropped — see `crate::proto`.
fn failed(e: RmError) -> Reply {
    Reply::Failed(match e {
        RmError::InsufficientPermissions => WireError::InsufficientPermissions,
        RmError::BadHandle(h) => WireError::BadHandle(h.raw()),
        RmError::NoMemory => WireError::NoMemory,
        RmError::Interrupted => WireError::Interrupted,
        RmError::Other(status) => WireError::Other(status),
        // #102: a child backend cannot produce `PlacementRefused` — the check that mints
        // it lives in the PARENT's `Worker::execute`, above the wire. Listed explicitly
        // rather than caught by a wildcard, so adding a variant stays a compile error.
        RmError::Wedged | RmError::ForeignHandle { .. } | RmError::PlacementRefused { .. } => {
            WireError::Other(crate::rm::NOT_ON_THIS_RUNG)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::LoopbackShared;

    /// ★★ **A peer-supplied read length is bounded BEFORE a buffer exists.**
    ///
    /// `len` arrives on the wire. `vec![0u8; len]` with an unchecked `len` is a remote
    /// allocation, and the fact that the peer is our own parent is not a reason to write
    /// code that trusts a length — boundary 2 runs in both directions. The refusal is a
    /// named status, so it reads as a refusal and never as an empty page.
    #[test]
    fn an_oversized_fabricated_aperture_read_is_refused_before_anything_is_allocated() {
        let shared = LoopbackShared::new(ParkVerb::Nothing).expect("pipe");
        let mut rm = LoopbackRm::new(IsolateId::new(1, GpuId(0)), shared).expect("dup");

        assert_eq!(
            execute(
                &mut rm,
                Request::FbRead {
                    phys: 0,
                    len: (FB_READ_MAX as u64) + 1,
                },
            ),
            Reply::Failed(WireError::Other(FB_READ_TOO_LARGE)),
        );
        assert_eq!(
            execute(
                &mut rm,
                Request::FbRead {
                    phys: 0,
                    len: 1 << 24
                }
            ),
            Reply::Failed(WireError::Other(FB_READ_TOO_LARGE)),
            "…and comfortably past the bound, not only one byte past it"
        );

        // …while an ordinary read reaches the backend, which honestly maps nothing.
        assert_eq!(
            execute(
                &mut rm,
                Request::FbRead {
                    phys: 0x1000,
                    len: 4096,
                },
            ),
            Reply::FbBytes {
                covered: false,
                bytes: Vec::new(),
            },
            "a MISS is a MISS on the wire — it is not an error and it is not zeros"
        );
    }

    #[test]
    fn a_complete_argument_list_parses() {
        let args = ChildArgs::parse(
            [
                "--proc",
                "3",
                "--gpu",
                "1",
                "--workers",
                "4",
                "--rm",
                "loopback",
            ]
            .map(String::from),
        )
        .expect("parse");
        assert_eq!(
            args,
            ChildArgs {
                proc: 3,
                gpu: 1,
                workers: 4,
                rm: RmMode::Loopback,
                park: ParkVerb::Nothing,
            }
        );
        assert_eq!(args.isolate(), IsolateId::new(3, GpuId(1)));
    }

    #[test]
    fn every_missing_or_malformed_argument_is_refused_by_name() {
        let base = [
            "--proc",
            "1",
            "--gpu",
            "0",
            "--workers",
            "2",
            "--rm",
            "real",
        ];
        assert!(ChildArgs::parse(base.map(String::from)).is_ok());
        for drop_pair in 0..4 {
            let mut v: Vec<String> = base.iter().map(|s| (*s).to_owned()).collect();
            v.drain(drop_pair * 2..drop_pair * 2 + 2);
            assert!(
                ChildArgs::parse(v).is_err(),
                "argument pair {drop_pair} must be required"
            );
        }
        assert_eq!(
            ChildArgs::parse(
                [
                    "--proc",
                    "x",
                    "--gpu",
                    "0",
                    "--workers",
                    "1",
                    "--rm",
                    "real"
                ]
                .map(String::from)
            ),
            Err("--proc x".to_owned())
        );
        assert_eq!(
            ChildArgs::parse(["--rm"].map(String::from)),
            Err("--rm needs a value".to_owned())
        );
        assert_eq!(
            ChildArgs::parse(["--wat", "1"].map(String::from)),
            Err("unknown flag --wat".to_owned())
        );
    }

    /// A pool of zero is a configuration error, refused at the door rather than producing
    /// an isolate that can never answer.
    #[test]
    fn a_zero_width_pool_is_refused() {
        assert!(
            ChildArgs::parse(
                [
                    "--proc",
                    "1",
                    "--gpu",
                    "0",
                    "--workers",
                    "0",
                    "--rm",
                    "real"
                ]
                .map(String::from)
            )
            .is_err()
        );
    }

    /// ★ The child never claims a namespace. A handle it receives carries
    /// [`IsolateId::NONE`], so nothing on this side can fabricate provenance.
    #[test]
    fn the_child_stamps_no_namespace_on_a_handle_it_receives() {
        assert_eq!(raw(0), HostHandle::NULL);
        let h = raw(0x1234);
        assert_eq!(h.isolate(), IsolateId::NONE);
        assert_eq!(h.raw(), 0x1234);
    }

    /// The two variants a backend cannot produce are still mapped, loudly, rather than
    /// dropped — a `Reply` that silently became `Unit` would report success for a failure.
    #[test]
    fn the_unreachable_error_variants_still_become_a_failure() {
        for e in [
            RmError::Wedged,
            RmError::ForeignHandle {
                handle: HostHandle::NULL,
                worker_isolate: IsolateId::NONE,
            },
        ] {
            assert!(matches!(failed(e), Reply::Failed(WireError::Other(_))));
        }
    }

    #[test]
    fn every_rm_error_that_has_a_wire_form_keeps_its_identity() {
        let iso = IsolateId::new(1, GpuId(0));
        for e in [
            RmError::InsufficientPermissions,
            RmError::NoMemory,
            RmError::Interrupted,
            RmError::Other(0x23),
        ] {
            match failed(e) {
                Reply::Failed(w) => assert_eq!(w.into_rm_error(iso), e),
                other => panic!("expected a failure, got {other:?}"),
            }
        }
    }
}
