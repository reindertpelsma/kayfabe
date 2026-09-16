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

use crate::export::ChildExports;
use crate::fdcross::{CrossedFd, FdFrameError, FdOrigin, read_frame_with_fds, write_frame_with_fds};
use crate::guestram::GuestRamPlane;
use crate::isolate::{
    CONTROL_FD, GUEST_RAM_FD, PARK_WITNESS_FD, RmMode, WORKER_FD_BASE, decode_control,
};
use crate::loopback::{LoopbackRm, LoopbackShared, ParkVerb};
use crate::proto::{
    EXPORT_SOURCE_FABRICATED, EXPORT_SOURCE_HOST_DEVICE, Envelope, Reply, Request, WireError,
    engine_from_code, prot_code, prot_from_code, read_frame, write_frame,
};
use crate::rm::{HostRmBackend, RmConnection};
use kayfabe_arch::ids::{ClassId, ControlCmd, GpuId, GpuVa};
use kayfabe_isolate::{
    CeExecutor, CeSource, CeSubCopy, ExportRequest, ExportSource, GuestRamGrant, GuestRamMapped,
    HostHandle, HostedObject, IsolateId, RmBackend, RmError,
};
use kayfabe_linux_raw::sandbox::{self, SandboxPolicy};
use kayfabe_linux_raw::{
    HostPageSize, ThreadId, adopt_inherited_fd, current_thread_id, install_break_handler,
    interrupt_thread,
};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::sync::{Arc, Mutex};

/// How the child was invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildArgs {
    /// The owning proc's id.
    pub proc: u32,
    /// ★★★ Whether this isolate is the VM-lifetime scratchpad and must bring CUDA up
    /// **before** `sandbox::enter` — `THE_CONSTRAINTS.md` §w724d.
    ///
    /// ⊘ Carried explicitly rather than inferred from `proc == u32::MAX`, so the child's own
    /// view of its configuration is complete and it can SAY which ordering it ran. An isolate
    /// that has to guess what it is cannot report it.
    pub cuda_walk: bool,
    /// ★★★★★ **CONSTRAINT 26** — whether this isolate allocates **bare** address spaces: a
    /// `FERMI_VASPACE_A` with no `NV01_MEMORY_VIRTUAL` range over it, so it can bind
    /// channels to the space and map nothing into it.
    ///
    /// ⊘ Carried explicitly rather than inferred from `proc != u32::MAX`, for
    /// [`ChildArgs::cuda_walk`]'s reason and one more: the scratchpad's exemption is the
    /// composition root's decision, and an isolate that re-derived it would be a second
    /// place that decision lives.
    pub bare_vaspaces: bool,
    /// The GPU this isolate is the sandbox for.
    pub gpu: u32,
    /// Pool width — one worker thread and one socket each.
    pub workers: usize,
    /// Which RM to speak to.
    pub rm: RmMode,
    /// Which verb parks forever (loopback only).
    pub park: ParkVerb,
    /// ★★★ How many bytes of guest RAM were granted on
    /// [`crate::isolate::GUEST_RAM_FD`], or `0` for "no grant was made".
    ///
    /// ⊘ The extent is told to the child rather than discovered by it — see
    /// [`crate::guestram::GuestRamPlane`]'s `bytes` field for why a second source of truth
    /// for this one number is the hazard.
    pub guest_ram_bytes: u64,
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
        let mut cuda_walk = false;
        let mut bare_vaspaces = false;
        let mut gpu = None;
        let mut workers = None;
        let mut rm = None;
        let mut park = ParkVerb::Nothing;
        let mut guest_ram_bytes: u64 = 0;
        let mut it = args.into_iter();
        while let Some(flag) = it.next() {
            let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--proc" => proc = Some(value.parse().map_err(|_| format!("--proc {value}"))?),
                // ★★★ §w724d: this isolate brings CUDA all the way up BEFORE it is
                // sandboxed. ⊘ Refused rather than defaulted if it names neither state —
                // a typo must not silently produce the ordinary ordering in a process that
                // was spawned from the dynamically-linked image.
                "--cuda-walk" => {
                    cuda_walk = match value.as_str() {
                        "on" => true,
                        "off" => false,
                        other => return Err(format!("--cuda-walk {other}")),
                    };
                }
                // ★★★★★ **CONSTRAINT 26** — does this isolate allocate BARE `FERMI_VASPACE_A`s,
                // i.e. spaces with no `NV01_MEMORY_VIRTUAL` range it could map through?
                // ⊘ Refused rather than defaulted if it names neither state, exactly as
                // `--cuda-walk` is: a typo here silently produces the PRE-§26 ownership, in
                // which the isolate holds an `hMemory` for guest video memory.
                "--bare-vaspaces" => {
                    bare_vaspaces = match value.as_str() {
                        "on" => true,
                        "off" => false,
                        other => return Err(format!("--bare-vaspaces {other}")),
                    };
                }
                "--gpu" => gpu = Some(value.parse().map_err(|_| format!("--gpu {value}"))?),
                "--workers" => {
                    workers = Some(value.parse().map_err(|_| format!("--workers {value}"))?);
                }
                "--rm" => rm = Some(RmMode::parse(&value).ok_or(format!("--rm {value}"))?),
                "--park" => park = ParkVerb::parse(&value).ok_or(format!("--park {value}"))?,
                "--guest-ram-bytes" => {
                    guest_ram_bytes = value
                        .parse()
                        .map_err(|_| format!("--guest-ram-bytes {value}"))?;
                }
                other => return Err(format!("unknown flag {other}")),
            }
        }
        let workers = workers.ok_or("--workers is required")?;
        if workers == 0 {
            return Err("--workers 0 is an isolate that can never issue a verb".to_owned());
        }
        Ok(ChildArgs {
            proc: proc.ok_or("--proc is required")?,
            cuda_walk,
            bare_vaspaces,
            gpu: gpu.ok_or("--gpu is required")?,
            workers,
            rm: rm.ok_or("--rm is required")?,
            park,
            guest_ram_bytes,
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

    // ★ One export table per ISOLATE, not per worker. A backing is an isolate-scoped
    // resource; a per-worker table would make a token's meaning depend on which pool slot
    // happened to serve the request, and the parent has no way to know which that was.
    let exports = Arc::new(ChildExports::new());

    // ★★★ Guest RAM, adopted from the FIXED number the parent granted it on — and only
    // when the parent said it granted one. ⊘ An absent descriptor here is the EXPECTED
    // state for a VM launched without a shared memory backing, not a degradation to
    // tolerate silently: the parent's grant and this adoption are conditional on the same
    // predicate (`--guest-ram-bytes`), so a descriptor that is missing when the parent said
    // it granted one is a genuine startup failure and ends the isolate.
    let guest_ram = if args.guest_ram_bytes == 0 {
        None
    } else {
        match adopt_inherited_fd(GUEST_RAM_FD) {
            Ok(fd) => Some(Arc::new(GuestRamPlane::new(
                fd,
                args.guest_ram_bytes,
                HostPageSize::query(),
            ))),
            Err(e) => {
                eprintln!("kayfabe-isolate: the guest-RAM descriptor was not granted: {e}");
                return 2;
            }
        }
    };

    // ★ Build the backends BEFORE announcing readiness, so a bring-up failure is reported
    // on the hello frame rather than on the first guest operation.
    // ★★★★★ ONE join table per isolate, built HERE and cloned into every worker's backend.
    // See `crate::fbjoin`: an isolate is a pool, and a per-worker table is a bug no
    // single-worker test can see.
    let fb_joins = Arc::new(crate::fbjoin::FbJoinTable::new());
    let backends = match build_backends(args, id, &exports, guest_ram.as_ref(), &fb_joins) {
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
        let exports = Arc::clone(&exports);
        threads.push(std::thread::spawn(move || {
            worker_loop(i, sock, backend, &slots, &exports, id);
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

fn build_backends(
    args: &ChildArgs,
    id: IsolateId,
    exports: &Arc<ChildExports>,
    guest_ram: Option<&Arc<GuestRamPlane>>,
    fb_joins: &Arc<crate::fbjoin::FbJoinTable>,
) -> Result<Vec<Box<dyn RmBackend>>, String> {
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
            // ★★★★★ **§w724d STEP 2 — CUDA COMES UP HERE, BEFORE THE SANDBOX.**
            //
            // > *"before entering mount namespace and chroot, it first opens libcuda and
            // > inits and loads the entire channel and PTX ensure its running; then it drops
            // > privileges as usual"* — owner, 2026-09-14.
            //
            // ★ Why it works at all: **open fds, existing mappings, the CUDA context and the
            // loaded module all survive a namespace change.** Only PATH LOOKUPS do not. So
            // everything lazy has to be walked while paths still exist — `cuInit`, device
            // enumeration, `cuCtxCreate`, `cuModuleLoadData` (the PTX JIT), every allocation
            // and a real launch that reads a report back.
            //
            // ⚠ **THE HONEST COST, stated at the site rather than only in the design doc:**
            // this process has a FULL FILESYSTEM VIEW for the duration of this call. It is
            // bounded — only our init and NVIDIA's init run in it, before any guest data is
            // touched, at the same trust level as VMM startup — and it is a real change from
            // "sandboxed before anything runs". Constraint 20's argument must be read as
            // *"the process ends with the same reach"*, not *"it never had more"*.
            //
            // ⊘ **A failure here does NOT fail the isolate.** The reservation and the RM
            // plane are what the VM needs to boot; the walk kernel is increment 4's gate and
            // nothing depends on it yet. So the outcome is RECORDED and the isolate carries
            // on — which is also what makes the census able to say *why* rather than leaving
            // the parent with a dead isolate and no diagnosis.
            #[cfg(feature = "cuda-scratchpad")]
            if args.cuda_walk {
                crate::cudawalk::bring_up_before_sandbox();
            }
            #[cfg(not(feature = "cuda-scratchpad"))]
            if args.cuda_walk {
                // ⊘ Named, not silent: a boot that ASKED for the CUDA scratchpad and got a
                // binary built without it must say so, or the absent census line reads as
                // "the gate was off".
                eprintln!(
                    "kayfabe-isolate: ⊘ --cuda-walk=on but this binary was built WITHOUT the \
                     `cuda-scratchpad` feature; no CUDA was brought up and none will be."
                );
            }
            let dev = sandbox::enter(&SandboxPolicy::for_gpu(args.gpu)).map_err(|e| {
                // ⊘⊘ **NAME THE SUSPICION, because it is specific and it is new.** CUDA's
                // bring-up SPAWNS DRIVER THREADS, and `unshare(CLONE_NEWUSER)` — the first
                // thing `sandbox::enter` tries — is refused by the kernel to a multi-threaded
                // process. The fallback arm (`unshare` of mount/net/ipc/uts using the
                // CAP_SYS_ADMIN this child already has from the user namespace its PARENT
                // created at `clone`) is what must carry it.
                //
                // ⚠ If that fallback ever stops working, the failure arrives here as a bare
                // `EINVAL` on a call that has worked on every boot for months, and the last
                // thing changed would be in a different file. Saying so costs one line.
                if args.cuda_walk {
                    format!(
                        "the isolate sandbox could not be built: {e} — ⚠ this is the CUDA \
                         scratchpad isolate, and CUDA's bring-up ran BEFORE this call and \
                         spawns driver threads. `unshare(CLONE_NEWUSER)` is refused to a \
                         multi-threaded process, so this path depends on the fallback arm \
                         (mount/net/ipc/uts under the CAP_SYS_ADMIN inherited from the \
                         parent's clone). Suspect that before suspecting the policy."
                    )
                } else {
                    format!("the isolate sandbox could not be built: {e}")
                }
            })?;
            // ★★★ §w724d STEP 3 IS DONE (namespace + pivot_root + privilege drop). NOW the
            // two probes that the warm-up above cannot stand in for.
            #[cfg(feature = "cuda-scratchpad")]
            if args.cuda_walk {
                crate::cudawalk::probe_after_sandbox();
            }
            // ★ #156 — the host board's class profile, PINNED. See
            // `kayfabe_chips::host_classes::pinned_host_classes`: this process does not
            // ask the device what generation it is.
            //
            // The pin is GA10x because that is the only generation the rungs below it
            // have ever run on — the CE copy and the `userdOffset` failure shape were
            // both measured on RTX 3060 / 580.159.04, 2026-07-30, and are cited at their
            // call sites in `rm.rs` (`alloc_channel_on`'s `userd_offset_0`,
            // `HostRmBackend::ce_copy`). Nothing has been measured on Ada or Hopper:
            // those profiles are INFERRED from `ogkm`'s per-chip class tables
            // (`src/nvidia/generated/g_gpu_class_list.c`) and compile only.
            let conn = Arc::new(
                RmConnection::open(&dev, GpuId(args.gpu), kayfabe_chips::pinned_host_classes())
                    .map_err(|e| e.to_string())?,
            );
            Ok((0..args.workers)
                .map(|_| {
                    Box::new(
                        HostRmBackend::new(id, Arc::clone(&conn), Arc::clone(exports))
                            .with_guest_ram(guest_ram.map(Arc::clone))
                            .with_fb_joins(Arc::clone(fb_joins))
                            // ★★★★★ **CONSTRAINT 26** — see `ChildArgs::bare_vaspaces`.
                            .with_bare_vaspaces(args.bare_vaspaces),
                    ) as Box<dyn RmBackend>
                })
                .collect())
        }
        RmMode::Loopback => {
            // ★ Adopt the park witness only when a park is armed — its grant is conditional
            // on exactly the same predicate in the parent, so an absent descriptor here is
            // the expected state, not a degradation to tolerate silently.
            let witness = if args.park == ParkVerb::Nothing {
                None
            } else {
                let fd = adopt_inherited_fd(PARK_WITNESS_FD)
                    .map_err(|e| format!("adopting the park witness: {e}"))?;
                Some(std::io::PipeWriter::from(fd))
            };
            let shared =
                LoopbackShared::new(args.park, witness).map_err(|e| format!("park pipe: {e}"))?;
            let mut out: Vec<Box<dyn RmBackend>> = Vec::with_capacity(args.workers);
            for _ in 0..args.workers {
                out.push(Box::new(
                    LoopbackRm::new(id, Arc::clone(&shared), Arc::clone(exports))
                        .map_err(|e| format!("park pipe dup: {e}"))?
                        .with_guest_ram(guest_ram.map(Arc::clone))
                        .with_fb_joins(Arc::clone(fb_joins)),
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

/// ★★★ w321 — the four buckets the guest-RAM pin chain is made of, plus everything else.
///
/// ⊘ Four and not twenty: `VerbPlan::PinGuestRam` is exactly `MapGuestRam` →
/// `DescribeGuestRam` → `MapGpuVa`, three separate requests over the socket, and those are
/// the only ones the drain issues 13 313 times. A per-variant table would report thirty
/// zeros beside the three numbers that matter.
const W321_BUCKETS: usize = 4;
const W321_NAMES: [&str; W321_BUCKETS] =
    ["map_guest_ram", "describe_guest_ram", "map_gpu_va", "other"];

fn w321_bucket(r: &Request) -> usize {
    match r {
        Request::MapGuestRam { .. } => 0,
        Request::DescribeGuestRam { .. } => 1,
        Request::MapGpuVa { .. } => 2,
        _ => 3,
    }
}

thread_local! {
    /// `(count, nanos)` per bucket, and the running total request count.
    ///
    /// ⊘ Thread-local, like the parent's `IPC_TOTALS` and for the same reason: one worker
    /// thread serves one socket, a lock here would add to the quantity being measured, and
    /// another thread's traffic is simply invisible rather than mixed in.
    static W321_STAT: std::cell::RefCell<([(u64, u128); W321_BUCKETS], u64)> =
        const { std::cell::RefCell::new(([(0, 0); W321_BUCKETS], 0)) };
}

/// How many served requests between cumulative reports.
///
/// ⊘ CUMULATIVE and never reset — the parent's counter is monotonic too, so the two align
/// by subtraction over any interval a reader picks. A resettable counter here would let the
/// two sides disagree about which interval they are describing.
const W321_REPORT_EVERY: u64 = 1000;

fn w321_note(index: usize, bucket: usize, nanos: u128) {
    W321_STAT.with(|c| {
        let mut s = c.borrow_mut();
        s.0[bucket].0 += 1;
        s.0[bucket].1 += nanos;
        s.1 += 1;
        if s.1 % W321_REPORT_EVERY != 0 {
            return;
        }
        let mut parts = String::new();
        for (i, name) in W321_NAMES.iter().enumerate() {
            let (n, ns) = s.0[i];
            // ⊘ An UNSERVED bucket prints `⊘UNMEASURED`, never `0us` — a mean over zero
            // samples is not zero, and this tree has paid for that spelling repeatedly.
            if n == 0 {
                parts.push_str(&format!(" {name}[n=0 ⊘UNMEASURED]"));
            } else {
                parts.push_str(&format!(
                    " {name}[n={n} tot={}us mean={}us]",
                    ns / 1000,
                    ns / 1000 / u128::from(n)
                ));
            }
        }
        eprintln!(
            "kayfabe-isolate: W321CHILD worker={index} served={} — CHILD-SIDE SERVICE TIME \
             (`serve_one` only; the frame read/write and the socket are OUTSIDE it, so \
             parent_ipc_us MINUS these is the transport):{parts}",
            s.1
        );
    });
}

fn worker_loop(
    index: usize,
    sock: UnixStream,
    mut backend: Box<dyn RmBackend>,
    slots: &[Mutex<SlotState>],
    exports: &ChildExports,
    // ⊘ **This isolate's own identity, threaded rather than derived.** `AdoptBirthClient`
    // attributes the descriptors it receives to `(minted_by_proc, THIS gpu)`, and the GPU
    // has to come from somewhere. Taking it from the frame would be a second routing
    // decision on a path that already has one — the
    // `a_second_source_of_truth_beside_a_complete_value` defect w746 fixed in
    // `vaspace_handover` — so it comes from the process's own arguments, the authority.
    id: IsolateId,
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
        // ★★★★★ **w753 / CONSTRAINT 32 — THE READER NOW SUPPLIES A CONTROL BUFFER.**
        //
        // Until route K this was a plain `read_frame`, and `proto.rs` said so by name:
        // *"the fd-IN gap on the request path is REAL (the child's reader is `read_frame`,
        // with no control buffer at all)"*. [`Request::AdoptBirthClient`] closes it.
        //
        // ⊘⊘ **AND THE FAILURE MODE OF GETTING THIS WRONG IS SILENCE, WHICH IS WHY IT IS
        // HERE AND NOT BEHIND A FLAG.** A `recvmsg` with no control buffer does not refuse a
        // descriptor — the kernel **closes it and delivers the bytes perfectly**. The verb
        // would have returned `Ok`, the census would have counted a hand-over, and the far
        // side would hold a client handle naming a session it cannot reach. So
        // `serve_one` refuses `AdoptBirthClient` with an empty `fds` **by its own name**
        // rather than trusting this line to be right. See
        // `a_reader_without_a_control_buffer_loses_the_descriptor_and_reports_nothing`,
        // which puts both readers on one wire and shows the only observable that differs.
        //
        // ⚠ The allowance is a CONSTANT and cannot be per-verb: this is one reader for every
        // request and it has not decoded the body yet. So the allowance is the maximum any
        // verb may carry, and the per-verb rule — *"every request but one carries none"* —
        // is enforced below, after the decode, where it can name the verb it refused.
        let mut fds: Vec<OwnedFd> = Vec::new();
        match read_frame_with_fds(sock.as_fd(), &mut buf, &mut fds, REQUEST_MAX_FDS) {
            Ok(true) => {}
            // The parent closed, or abandoned this channel (§7.5). Either way there is
            // nothing left to serve.
            Ok(false) => return,
            // A cancel that arrived while this worker was BETWEEN verbs. There is nothing
            // to interrupt, so it is spent here rather than left to break the next read.
            // ⊘ Spelled through `RawError::is_interrupted` rather than `io::ErrorKind`
            // because `read_frame_with_fds` reports at a frame boundary and retries
            // mid-frame itself — the same rule, restated in the other reader's vocabulary.
            // A reader that dropped this arm would turn every cancel-between-verbs into a
            // dead worker.
            Err(FdFrameError::Os(e)) if e.is_interrupted() => continue,
            Err(_) => return,
        }
        let Ok(envelope) = Envelope::decode(&buf) else {
            return;
        };
        {
            let mut s = slots[index].lock().unwrap_or_else(|e| e.into_inner());
            s.txn = envelope.txn;
        }
        // ★★★★★ **w321 — THE CHILD-SIDE HALF OF THE TIMESTAMP DECOMPOSITION.**
        //
        // The parent already owns the OTHER half: `ProxyRmBackend::call`'s `IPC_TOTALS`
        // counts the **round trip** — request written → reply read — which INCLUDES
        // everything measured here. Neither counter alone can answer *"is the 225 µs/row
        // the socket or the ioctl?"*, and that question decides w321's whole fix: if the
        // cost is transport, one request carrying N rows removes it and **no physical
        // contiguity is needed**; if the cost is the ioctl, only coalescing or a batched RM
        // verb can help. ⊘ A hypothesis that fits the magnitude is not a mechanism — hence
        // two independent brackets on the two sides of the same socket, subtractable.
        //
        // ⊘ It times `serve_one`, which is the backend call plus the wire-vocabulary match
        // and nothing else — the frame read and the frame write are deliberately OUTSIDE
        // it, because those are the transport this is meant to be subtracted from.
        let t0 = std::time::Instant::now();
        let bucket = w321_bucket(&envelope.request);
        let (reply, carried) = serve_one(&mut *backend, envelope.request, exports, fds, id);
        w321_note(index, bucket, t0.elapsed().as_nanos());
        {
            let mut s = slots[index].lock().unwrap_or_else(|e| e.into_inner());
            s.txn = 0;
        }
        // ★★ A descriptor rides the frame's FIRST byte, so it goes on the same single
        // `sendmsg` the body does. Only an export reply ever carries one; every other reply
        // takes the plain writer, so the fd-carrying path cannot be reached by a verb that
        // was not asked for a backing.
        let wrote = match &carried {
            Some(fd) => write_frame_with_fds(sock.as_fd(), &reply.encode(), &[fd.as_fd()]).is_ok(),
            None => write_frame(&mut sock, &reply.encode()).is_ok(),
        };
        if !wrote {
            return;
        }
    }
}

/// ★★★★★ **CONSTRAINT 32 — HOW MANY DESCRIPTORS A REQUEST MAY CARRY, AT MOST.**
///
/// Two, and they are [`Request::AdoptBirthClient`]'s: a second `/dev/nvidiactl` and the
/// matching per-GPU node, bound to each other by `NV_ESC_REGISTER_FD` before they were sent.
/// **Both** are needed and neither is padding — the control node is what an escape is issued
/// on, and the per-GPU node keeps the session's GPU binding alive now that the isolate that
/// opened them has closed its own copies.
///
/// ⊘ **It is a CONSTANT, not a per-verb allowance, and that is a property of the reader
/// rather than a choice.** One reader serves every request and it has not decoded the body
/// when it must size the control buffer. ⇒ the constant is the maximum any verb may carry,
/// and the per-verb rule lives in [`serve_one`], after the decode, where a refusal can name
/// the verb it refused.
///
/// ⚠ **Expiry condition** (§w724g): back to `0` — and the reader back to `read_frame` — in
/// the same change that retires route K, if it is ever retired.
const REQUEST_MAX_FDS: usize = 2;

/// ★★★★★ **CONSTRAINT 32 — TAKE THE BIRTH CLIENT'S DESCRIPTORS AND PROVE THEY ARE WHAT THE
/// PROTOCOL PROMISED.**
///
/// Three checks, all fail-closed, before the backend is told anything:
///
/// 1. **The descriptors arrived at all.** ⊘ This is the check that catches a reader with no
///    control buffer — a `recvmsg` without one does not refuse a descriptor, it **closes it
///    and delivers the bytes perfectly**, so the only evidence of that bug is an empty
///    `fds` on a verb that cannot work without them. It is refused by a code of its own so
///    that *"the transport dropped them"* can never read as *"RM said no"*.
/// 2. **They are character devices**, checked against the kernel by [`CrossedFd::adopt`] and
///    not against the sender's claim. The next thing anyone does with one is issue an
///    `ioctl` naming a foreign client; a regular file here is a protocol confusion.
/// 3. **The client handle is not null.** A zero `hRoot` is the one value RM interprets
///    rather than refuses.
///
/// ⊘ **The provenance recorded is [`FdOrigin::BirthClient`]**, so if either descriptor is
/// ever lent onward it meets the one-target rule rather than `FdOrigin::Isolate`'s. It is
/// recorded here — at adoption — because that is the only moment the information exists: a
/// later caller has an `OwnedFd` and no way to recover where it came from.
fn adopt_birth_client(
    rm: &mut dyn RmBackend,
    client: u64,
    minted_by_proc: u32,
    fds: Vec<OwnedFd>,
    id: IsolateId,
) -> (Reply, Option<OwnedFd>) {
    let Ok([ctl, node]) = <[OwnedFd; 2]>::try_from(fds) else {
        kayfabe_util::lock_safe_eprintln!(
            "kayfabe-isolate-host: ⊘⊘⊘ CONSTRAINT 32 REFUSED — AdoptBirthClient arrived \
             without its two descriptors. ⚠ Read this as a TRANSPORT defect, not an RM one: \
             a recvmsg with no control buffer CLOSES what the sender attached and delivers \
             the body intact, so an empty `fds` here is what a reader that forgot \
             `read_frame_with_fds` looks like. The client handle on its own reaches no RM."
        );
        return (
            failed(RmError::Other(crate::rm::BIRTH_CLIENT_NO_DESCRIPTORS)),
            None,
        );
    };
    if client == 0 {
        kayfabe_util::lock_safe_eprintln!(
            "kayfabe-isolate-host: ⊘⊘ CONSTRAINT 32 REFUSED — AdoptBirthClient named client \
             0x0. A null hRoot is the one handle RM interprets instead of refusing."
        );
        return (failed(RmError::Other(crate::rm::BIRTH_CLIENT_NULL_HANDLE)), None);
    }
    // ⊘ The isolate the descriptors are attributed to. The GPU is THIS backend's own — a
    // birth client is per-`(proc, gpu)` exactly as an isolate is, and a frame that could
    // name another GPU would be a second routing decision on a path that already has one.
    let minted_by = IsolateId::new(minted_by_proc, id.gpu());
    let origin = FdOrigin::BirthClient { minted_by };
    let (ctl, node) = match (
        CrossedFd::adopt(ctl, origin, kayfabe_linux_raw::DescriptorKind::CharDevice),
        // ⊘ Adopted in the same expression so a refusal of EITHER closes BOTH: each `adopt`
        // takes its descriptor by value, and the tuple is dropped whole.
        CrossedFd::adopt(node, origin, kayfabe_linux_raw::DescriptorKind::CharDevice),
    ) {
        (Ok(c), Ok(n)) => (c, n),
        (c, n) => {
            kayfabe_util::lock_safe_eprintln!(
                "kayfabe-isolate-host: ⊘⊘ CONSTRAINT 32 REFUSED — a birth-client descriptor \
                 is not a character device. ctl={:?} node={:?}. Checked against the KERNEL, \
                 not against the sender's claim, because the next thing done with one is an \
                 ioctl naming a foreign client.",
                c.as_ref().err(),
                n.as_ref().err(),
            );
            return (
                failed(RmError::Other(crate::rm::BIRTH_CLIENT_NOT_A_CHAR_DEVICE)),
                None,
            );
        }
    };
    match rm.adopt_birth_client(client, minted_by_proc, ctl.into_owned(), node.into_owned()) {
        Ok(()) => (Reply::Unit, None),
        Err(e) => (failed(e), None),
    }
}

/// One request, its reply, and **the descriptor that reply carries, if any**.
///
/// The split exists because exactly one verb's answer is a descriptor and everything else
/// is bytes. Threading an `Option<OwnedFd>` through [`execute`]'s twenty arms would put a
/// resource in nineteen places that have none; making the export arm a sibling keeps
/// [`execute`] a pure `Request -> Reply` function, which is what its own tests assert.
fn serve_one(
    rm: &mut dyn RmBackend,
    request: Request,
    exports: &ChildExports,
    fds: Vec<OwnedFd>,
    id: IsolateId,
) -> (Reply, Option<OwnedFd>) {
    // ★★★★★ **w753 / CONSTRAINT 32 — THE PER-VERB fd RULE, ENFORCED AFTER THE DECODE.**
    //
    // The reader's allowance is a constant because one reader serves every verb. This is
    // where the actual protocol rule lives: **exactly one request may carry descriptors**.
    // Refused by name, and `fds` is dropped on the refusal path, so whatever arrived is
    // CLOSED rather than leaked — the property `CrossedFd::adopt` has by taking its
    // descriptor by value, restated here for the frames that never reach it.
    //
    // ⊘ This mirrors `a_descriptor_on_a_message_that_may_not_carry_one_is_refused` on the
    // reply direction. Both directions get the rule because a peer is not obliged to be
    // well-behaved in either of them.
    if !fds.is_empty() && !matches!(request, Request::AdoptBirthClient { .. }) {
        kayfabe_util::lock_safe_eprintln!(
            "kayfabe-isolate-host: ⊘⊘ REFUSED — {} descriptor(s) arrived on a request that \
             may not carry one. Exactly ONE request may (`AdoptBirthClient`, constraint 32); \
             every other is bytes. The descriptors are closed here.",
            fds.len(),
        );
        return (
            failed(RmError::Other(crate::rm::FD_ON_A_BYTES_ONLY_REQUEST)),
            None,
        );
    }
    match request {
        // ★★★★★ **CONSTRAINT 32 — THE ONE REQUEST THAT CARRIES DESCRIPTORS DOWN.**
        Request::AdoptBirthClient {
            client,
            minted_by_proc,
        } => adopt_birth_client(rm, client, minted_by_proc, fds, id),
        Request::ExportBacking {
            source,
            memory,
            len,
            prot,
        } => export_backing(rm, source, memory, len, prot, exports),
        // ★★★★★ The SECOND request whose reply carries a descriptor. It is intercepted here
        // for `export_backing`'s reason exactly — `execute` is a pure `Request -> Reply`
        // function and a resource has no place in nineteen of its arms.
        Request::JoinFbLeaf {
            vas,
            len,
            at,
            phys,
            prot,
        } => join_fb_leaf(rm, vas, len, at, phys, prot, exports),
        // ★★★★★ **w629 — the THIRD request whose reply carries a descriptor**, and the only
        // one that carries a CHARACTER DEVICE. Intercepted here for the other two's reason:
        // `execute` is a pure `Request -> Reply` function and a resource has no place in
        // nineteen of its arms.
        Request::ExportUsermodeView { write } => export_usermode_view(rm, exports, write != 0),
        // ★★★★★ **RE-ISSUED 2026-09-14 — the FOURTH descriptor-carrying reply**, and the
        // second whose descriptor is a character device. Intercepted here for the reason the
        // other three are: `execute` is a pure `Request -> Reply` function and a resource has
        // no place in twenty of its arms.
        //
        // ⊘ `bar1_passthrough_device_local_host_visible.md` §4 item 1 — the owner's ruling of
        // 2026-09-14 granting decision (b) — is what authorises this verb's existence. The
        // orphan note that retired tags 25/12 is still right about those numbers.
        Request::ExportDeviceView {
            memory,
            offset,
            len,
            write,
        } => export_device_view(rm, memory, offset, len, write != 0, exports),
        other => (execute(rm, other), None),
    }
}

/// ★★★★★ **w629 — arm the isolate's OWN usermode page read-only and hand the node up.**
///
/// ⊘ The request names nothing, so there is nothing here to validate and nothing a caller can
/// steer. The backend arms its own object; this function only lends the node and reports the
/// length the VMM's `mmap` must use.
///
/// ⚠ The descriptor returned is a `/dev/nvidiaN` **character device**, which is exactly what
/// `export_backing`'s twin REFUSES on the far side — deliberately, because a backing must be a
/// `memfd`. ⇒ The VMM's caller for this verb must be its own, with the opposite kind check.
/// Sharing one would trade that refusal for a shortcut. See
/// `the_counter_page_and_the_device_view.md` §4c.
/// ★★★★★ **RE-ISSUED 2026-09-14 — arm a CPU view of the reserved object and hand the node up.**
///
/// The VMM asks for `[offset, offset+len)` of one of this isolate's RM objects; the backend
/// arms a fresh `/dev/nvidia<N>` with a one-shot `NV_ESC_RM_MAP_MEMORY` context for exactly
/// that range, and the node crosses on this reply's `SCM_RIGHTS`.
///
/// ⚠ **Unlike `export_usermode_view`, the caller NAMES AN OBJECT** — so the foreign-handle
/// gate matters here and does not there. It is applied one layer up, in
/// `kayfabe_isolate::Worker::export_device_view`, which refuses a handle from a sibling
/// isolate's namespace before this is ever reached.
fn export_device_view(
    rm: &mut dyn RmBackend,
    memory: u64,
    offset: u64,
    len: u64,
    write: bool,
    exports: &ChildExports,
) -> (Reply, Option<OwnedFd>) {
    let view = match rm.export_device_view(raw(memory), offset, len, write) {
        Ok(v) => v,
        Err(e) => return (failed(e), None),
    };
    // ⊘ `view.token` is the CHILD's index into its own table. The PARENT's token does not
    // come from here — it mints its own when it adopts the descriptor.
    //
    // ⊘⊘⊘ **w734 — but the child's DOES cross now, as `release_token`.** A release is executed
    // here, against this table, and before w734 the parent handed back a token of its own
    // minting. They coincide only because each mint is matched in order by one adopt, which
    // nothing stated and nothing checked.
    match exports.lend(view.token) {
        Ok(fd) => (
            Reply::DeviceViewNode {
                release_token: view.token,
                memory: view.memory.raw(),
                offset: view.offset,
                mmap_len: view.mmap_len,
            },
            Some(fd),
        ),
        // ⊘ The backend minted a token this table does not know — our bug, not the parent's.
        // Refused rather than answered with a descriptor-less reply, which would have the
        // parent adopt whatever descriptor arrived next.
        Err(_) => (
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            None,
        ),
    }
}

fn export_usermode_view(
    rm: &mut dyn RmBackend,
    exports: &ChildExports,
    write: bool,
) -> (Reply, Option<OwnedFd>) {
    let view = match rm.export_usermode_view(write) {
        Ok(v) => v,
        Err(e) => return (failed(e), None),
    };
    // ⊘ The token is the CHILD's; the parent mints its own when it adopts the descriptor. It
    // never crosses the wire, exactly as `export_backing`'s does not.
    match exports.lend(view.token) {
        Ok(fd) => (
            Reply::UsermodeView {
                mmap_len: view.mmap_len,
            },
            Some(fd),
        ),
        // ⊘ Same reasoning as `export_backing`'s twin: the backend minted a token this table
        // does not know, which is our bug and not the parent's. Refused rather than answered
        // with a descriptor-less `UsermodeView`, which would have the parent adopt whatever
        // descriptor arrived next.
        Err(_) => (
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            None,
        ),
    }
}

/// ★★★ The export arm: run the verb, and attach the backing it minted.
///
/// Three refusals happen **before** the backend is reached, and each is a wire value the
/// child must not guess at:
///
/// - an unknown `source` code — never defaulted, because defaulting to `Fabricated` would
///   answer a request for the card's bytes with a fresh page of zeros, and defaulting to
///   `HostDeviceMemory` would refuse a request that should have succeeded;
/// - an unknown `prot` code — never defaulted, and specifically never to read-write;
/// - a zero length, which `SharedRam::create` refuses anyway but which is worth naming
///   here rather than surfacing as a `memfd_create` errno.
fn export_backing(
    rm: &mut dyn RmBackend,
    source: u8,
    memory: u64,
    len: u64,
    prot: u8,
    exports: &ChildExports,
) -> (Reply, Option<OwnedFd>) {
    let Some(prot) = prot_from_code(prot) else {
        return (
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            None,
        );
    };
    let source = match source {
        EXPORT_SOURCE_FABRICATED => ExportSource::Fabricated,
        EXPORT_SOURCE_HOST_DEVICE => ExportSource::HostDeviceMemory {
            memory: raw(memory),
        },
        _ => {
            return (
                Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
                None,
            );
        }
    };
    let want = ExportRequest { source, len, prot };
    let backing = match rm.export_backing(want) {
        Ok(b) => b,
        Err(e) => return (failed(e), None),
    };
    // ★ `backing.token` is the CHILD's index into its own table. It does not go on the
    // wire: the parent mints its own when it adopts the descriptor (`export`'s module
    // docs). Here it is what says which descriptor to attach.
    match exports.lend(backing.token) {
        Ok(fd) => (
            Reply::Backing {
                offset: backing.offset,
                len: backing.len,
                prot: prot_code(backing.prot),
            },
            Some(fd),
        ),
        // The backend minted a token this table does not know, which is our own bug and
        // not the parent's. Reported as a failure rather than as a reply with no
        // descriptor: a `Backing` frame carrying nothing would have the parent adopt
        // whatever descriptor arrived next.
        Err(_) => (
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            None,
        ),
    }
}

/// ★★★★★ The join arm: run the chain, and attach the backing it minted.
///
/// [`export_backing`]'s twin one function up, and every refusal before the backend is
/// reached is the same value it refuses: an unknown `prot` code is never defaulted, and
/// specifically never to read-write.
///
/// ⊘ **The two arms are deliberately not merged.** They differ in what a wrong answer costs:
/// an export hands the VMM a page it may install wherever it likes, while a join has already
/// placed a host GPU mapping at an address the guest's own page tables bind. Sharing a body
/// would put one `if` between those two facts.
fn join_fb_leaf(
    rm: &mut dyn RmBackend,
    vas: u64,
    len: u64,
    at: u64,
    phys: u64,
    prot: u8,
    exports: &ChildExports,
) -> (Reply, Option<OwnedFd>) {
    let Some(prot) = prot_from_code(prot) else {
        return (
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            None,
        );
    };
    // ⊘ Decoded and then IGNORED, on purpose, and it is not dead: the join's backing is
    // minted read-write because the guest writes its own framebuffer through it. Refusing an
    // unrecognised code above is what stops a wire byte we do not understand from arriving as
    // a permission — the code being unused downstream does not make its validation optional.
    let _ = prot;
    let joined = match rm.join_fb_leaf(raw(vas), len, GpuVa(at), phys) {
        Ok(j) => j,
        Err(e) => return (failed(e), None),
    };
    // ★ `joined.backing.token` is the CHILD's index into its own table and does not go on the
    // wire; here it is what says which descriptor to attach. See `crate::export`.
    match exports.lend(joined.backing.token) {
        Ok(fd) => (
            Reply::JoinedBacking {
                offset: joined.backing.offset,
                len: joined.backing.len,
                prot: prot_code(joined.backing.prot),
                memory: joined.memory.raw(),
                host_va: joined.host_va,
            },
            Some(fd),
        ),
        // The backend minted a token this table does not know — our own bug, not the
        // parent's. Reported as a failure rather than as a reply with no descriptor: a
        // `JoinedBacking` frame carrying nothing would have the parent adopt whatever
        // descriptor arrived next. ⚠ The RM objects the chain built are LEAKED on this path
        // and that is stated rather than hidden: they are named only by a handle the parent
        // will never receive, so there is nothing left that can free them by name.
        Err(_) => (
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            None,
        ),
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
        // ★★★★★ **CONSTRAINT 26 — the bare space, and the three verbs that go with it.**
        //
        // ⊘ `AdoptVaSpace` is the only frame in this protocol that carries a client handle
        // this process did not mint, and it is **not** gated here: the gate is a type in
        // `crate::rm` (`handed_vaspace`), which answers `ADOPT_NOT_THE_SCRATCHPAD` before
        // any ioctl is built. Gating it here as well would be a second place the rule lives,
        // and the two would come apart the first time one of them was edited.
        Request::AllocVaSpaceBare => match rm.alloc_vaspace_bare() {
            Ok(b) => Reply::BareVaSpace {
                space: b.space.raw(),
                client: b.client,
            },
            Err(e) => failed(e),
        },
        Request::AdoptVaSpace { client, space } => handle(rm.adopt_vaspace(client, space)),
        Request::MapStoreSlice {
            vas,
            memory,
            offset,
            len,
            at,
        } => match rm.map_store_slice(raw(vas), raw(memory), offset, len, GpuVa(at)) {
            Ok(va) => Reply::Va(va),
            Err(e) => failed(e),
        },
        Request::UnmapStoreSlice { vas, at } => unit(rm.unmap_store_slice(raw(vas), GpuVa(at))),
        Request::VaSpaceHandover { space } => match rm.vaspace_handover(raw(space)) {
            Ok(b) => Reply::BareVaSpace {
                space: b.space.raw(),
                client: b.client,
            },
            Err(e) => failed(e),
        },
        Request::SubdeviceControl { cmd, mut payload } => {
            match rm.subdevice_control(ControlCmd(cmd), &mut payload) {
                Ok(()) => Reply::Payload(payload),
                Err(e) => failed(e),
            }
        }
        Request::AllocSysmem { len } => handle(rm.alloc_sysmem(len)),
        Request::AllocVidmem { len } => handle(rm.alloc_vidmem(len)),
        Request::ReserveGpga { len } => handle(rm.reserve_gpga(len)),
        // ★★★ A READ of what the pre-sandbox bring-up recorded. See
        // `Request::CudaWalkReport` — this verb cannot cause a bring-up, because by the time
        // a worker answers anything the isolate is already sandboxed.
        // ⊘ No descriptor: a release is a command, not an export. It goes through the plain
        // writer like every other verb.
        Request::ReleaseDeviceView { token } => match rm.release_device_view(token) {
            Ok(()) => Reply::Unit,
            Err(e) => failed(e),
        },
        // ⊘ Listed explicitly rather than swept into a catch-all: this verb's reply carries a
        // descriptor and is intercepted by `serve_one` BEFORE `execute` is reached. If it ever
        // arrives here the interception was lost, and answering it with a descriptor-less
        // reply would have the parent adopt whatever descriptor came next.
        Request::ExportDeviceView { .. } => {
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG))
        }
        // ★★★★★ The live shadow's two verbs. Like `CudaWalkReport` they do NOT go through
        // `rm`: the walk kernel belongs to the PROCESS (it was brought up before the sandbox,
        // on the startup path) and not to any RM connection. ⊘ Answered by name when the
        // feature is absent, so "this binary has no kernel" and "the kernel refused" stay
        // different facts.
        Request::WalkShadowStage { span, off, bytes } => {
            #[cfg(feature = "cuda-scratchpad")]
            {
                match crate::cudawalk::stage(span, off, &bytes) {
                    Ok(()) => Reply::Unit,
                    Err(e) => Reply::Failed(WireError::Other(e)),
                }
            }
            #[cfg(not(feature = "cuda-scratchpad"))]
            {
                let _ = (span, off, bytes);
                Reply::Failed(WireError::Other(kayfabe_isolate::NOT_A_WALK_SHADOW))
            }
        }
        Request::WalkShadowRun { pdbs } => {
            #[cfg(feature = "cuda-scratchpad")]
            {
                match crate::cudawalk::run(&pdbs) {
                    Ok(bytes) => Reply::Payload(bytes),
                    Err(e) => Reply::Failed(WireError::Other(e)),
                }
            }
            #[cfg(not(feature = "cuda-scratchpad"))]
            {
                let _ = pdbs;
                Reply::Failed(WireError::Other(kayfabe_isolate::NOT_A_WALK_SHADOW))
            }
        }
        Request::CudaWalkReport => {
            #[cfg(feature = "cuda-scratchpad")]
            {
                Reply::Payload(crate::cudawalk::report_line().into_bytes())
            }
            // ⊘ A NAMED absence, not an error: an isolate built without the feature is not
            // broken, it simply never ran CUDA — and the parent's census must be able to say
            // which of the two it is looking at.
            #[cfg(not(feature = "cuda-scratchpad"))]
            {
                Reply::Payload(
                    b"CUDA_WALK=ABSENT reason=\"this binary was built without the \
                      cuda-scratchpad feature\""
                        .to_vec(),
                )
            }
        }
        // ⊘ `Ok(0)` crosses as `Reply::Megabytes(0)`, NOT as a failure: *"nothing down to
        // the probe's floor could be reserved"* is an answer about this host, and the
        // parent must be able to tell it from *"the verb is not available"*.
        Request::LargestReservableMb { start_mb } => match rm.largest_reservable_mb(start_mb) {
            Ok(mb) => Reply::Megabytes(mb),
            Err(e) => failed(e),
        },
        Request::AllocChannel {
            vas,
            engine,
            hosting,
            adopt,
            err_notifier,
        } => match engine_from_code(engine) {
            // An engine code we do not recognise is a refusal, never a default — the GR-1
            // wrong-runlist class.
            None => Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            // ★★★ §16.106 — the guest's own declaration, rebuilt on THIS side of the wire
            // and handed to the adapter that reads it. Borrowed from `hosting`, which the
            // decode owns for exactly this call.
            Some(engine) => match rm.alloc_channel(
                raw(vas),
                engine,
                hosting.as_ref().map(|(class, params)| HostedObject {
                    class: ClassId(*class),
                    params,
                }),
                // ★★★★★ LEG A2 — rebuilt on THIS side of the wire, where the adapter that
                // lowers it runs. ⊘ The handle is re-validated by the adapter as one
                // `join_fb_leaf` minted; nothing here trusts the four integers.
                adopt.map(|(kind, a, b, ring_va, gp_fifo_va, gp_fifo_entries, userd)| {
                    kayfabe_isolate::AdoptedGuestRing {
                        ring: ring_provenance(kind, a, b),
                        ring_va,
                        gp_fifo_va,
                        gp_fifo_entries,
                        // ★★★★★ LEG B — rebuilt here for leg A2's reason, and re-validated
                        // by the adapter as an object `join_fb_leaf` minted. ⊘ Nothing on
                        // this side trusts the two integers either.
                        userd: userd.map(|(memory, offset)| kayfabe_isolate::AdoptedGuestUserd {
                            memory: raw(memory),
                            offset,
                        }),
                    }
                }),
                // ★★★★★ w288 — rebuilt on THIS side of the wire, where the adapter that puts
                // it in `hObjectError` runs. ⊘ `map`, never `unwrap_or(0)`: the presence byte
                // already carried the distinction across, and collapsing it here would throw
                // away the one thing the byte exists for. The handle is re-validated by the
                // adapter's own `narrow` as one this connection minted; nothing here trusts
                // the integer.
                err_notifier.map(raw),
            ) {
                Ok((h, token)) => Reply::HandleAndToken(h.raw(), token),
                Err(e) => failed(e),
            },
        },
        // ★★★★★ w393 — the birth-at-alloc verb, dispatched BY VARIANT to the trait method
        // whose type makes the adoption mandatory. ⊘ The five integers are rebuilt here and
        // re-validated by the adapter as objects `join_fb_leaf` minted, exactly as
        // `AllocChannel`'s `adopt` is; nothing on this side trusts them.
        Request::AllocChannelDeclared {
            vas,
            engine,
            declared_engine_type,
            adopt: (kind, a, b, ring_va, gp_fifo_va, gp_fifo_entries, userd),
            err_notifier,
        } => match engine_from_code(engine) {
            None => Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            Some(engine) => match rm.alloc_channel_declared(
                raw(vas),
                engine,
                declared_engine_type,
                kayfabe_isolate::AdoptedGuestRing {
                    ring: ring_provenance(kind, a, b),
                    ring_va,
                    gp_fifo_va,
                    gp_fifo_entries,
                    userd: userd.map(|(memory, offset)| kayfabe_isolate::AdoptedGuestUserd {
                        memory: raw(memory),
                        offset,
                    }),
                },
                err_notifier.map(raw),
            ) {
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
        // ★ Unreachable: [`serve_one`] intercepts this request, because its reply carries
        // a descriptor and this function's whole contract is `Request -> Reply`. Listed
        // explicitly rather than caught by a wildcard so that a future verb which also
        // carries a descriptor cannot be silently routed here and answered with bytes and
        // no fd — which the parent would read as a `Backing` naming nothing.
        Request::ExportBacking { .. }
        | Request::JoinFbLeaf { .. }
        | Request::ExportUsermodeView { .. } => {
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG))
        }
        // ★★★★★ **w753 / CONSTRAINT 32 — THE SAME RULE IN THE OTHER DIRECTION.** The arms
        // above carry a descriptor UP and cannot be answered here; this one carries two DOWN
        // and cannot either. Named explicitly for their reason exactly: reaching this arm
        // means `serve_one`'s intercept was removed or reordered, and a wildcard would answer
        // the request with a `Unit` while the descriptors it needed were already dropped — a
        // hand-over reported complete with nothing on the far end. ⊘ That is the
        // `a_check_that_reports_is_not_a_check_that_gates` shape; this refuses.
        Request::AdoptBirthClient { .. } => {
            Reply::Failed(WireError::Other(crate::rm::NOT_ON_THIS_RUNG))
        }
        // ★★★★★ **w380 — the alias, and it is NOT in the line above.** Its reply carries no
        // descriptor, so it is an ordinary `Request -> Reply` arm and belongs here. ⊘ The
        // distinction is load-bearing rather than tidy: routing it through `serve_one`'s
        // fd-carrying intercept would give its reply an allowance of one, and an allowance is
        // what the kernel checks instead of a `case`.
        Request::AliasFbLeaf { vas, len, at, phys } => {
            match rm.alias_fb_leaf(raw(vas), len, GpuVa(at), phys) {
                Ok(a) => Reply::Aliased {
                    memory: a.memory.raw(),
                    host_va: a.host_va,
                },
                Err(e) => failed(e),
            }
        }
        // ★★★ The instrument. `len` arrives on the wire, so it is bounded HERE, before a
        // buffer exists — `FbRead`'s argument, and it applies for the same reason: the fact
        // that the peer is our own parent is not a reason to write code that trusts a length.
        Request::FbJoinPeek {
            phys,
            len,
            poke,
            pattern,
        } => match usize::try_from(len) {
            Ok(n) if n <= FB_READ_MAX => {
                let mut buf = vec![0u8; n];
                let poke = (poke != 0).then_some(pattern);
                match rm.fb_join_peek(phys, &mut buf, poke) {
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
        Request::CeCopy {
            vas,
            dst,
            src,
            len,
            src_is_const,
            by_ours,
            rel_present,
            rel_va,
            rel_payload,
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
                // ★★★★★ w283 — rebuilt from the two fields only when the PRESENCE field
                // says so. ⊘ Never inferred from `rel_va != 0`: `0` is a legal GPU VA, and
                // this project has already paid once for reading a legal value as a blank
                // (`gpFifoOffset = 0`, `AdoptedGuestRing::gp_fifo_va`).
                guest_release: (rel_present != 0).then_some(kayfabe_isolate::CeGuestRelease {
                    va: rel_va,
                    payload: rel_payload,
                }),
            },
        )),
        // ★★★★★ The guest-RAM door. The child does not decide WHICH guest bytes it maps —
        // it reconstructs the VMM's grant verbatim and hands it to the plane. There is no
        // arm here that computes an offset or a length.
        Request::MapGuestRam { offset, len, prot } => {
            match crate::proto::prot_from_code(prot) {
                Some(prot) => {
                    match rm.map_guest_ram(GuestRamGrant::originated_by_the_vmm(offset, len, prot))
                    {
                        Ok(m) => Reply::Handle(m.region.raw()),
                        Err(e) => failed(e),
                    }
                }
                // ⊘ An unrecognised protection code is a REFUSAL, never a default. The
                // direction that fails open is the dangerous one: defaulting to read-write
                // would map guest pages writable on an authorization that said read-only,
                // which §3 names as a silent escalation.
                None => failed(RmError::Other(crate::rm::NOT_ON_THIS_RUNG)),
            }
        }
        Request::UnmapGuestRam { region, len } => unit(rm.unmap_guest_ram(GuestRamMapped {
            region: raw(region),
            len,
        })),
        // ★★★★★ `OS_DESCRIPTOR` over guest RAM. Same discipline as the door above: the
        // child names a MAPPING and computes no range. `len` is reconstructed from the
        // frame because `GuestRamMapped` carries it, and the backend's own plane is what
        // decides how many bytes are really there.
        Request::DescribeGuestRam { region, len } => {
            handle(rm.describe_guest_ram(GuestRamMapped {
                region: raw(region),
                len,
            }))
        }
    }
}

/// ★ The child's own view of a handle. It stamps [`IsolateId::NONE`] because **the child
/// has no business asserting a namespace**: the parent stamps every handle it receives with
/// the connection it asked on, so a provenance claim from this side would be a claim the
/// parent overrides anyway — and one that a compromised child could make.
/// ★★★★★ **CONSTRAINT 26 — rebuild the ring's provenance from its wire tag.**
///
/// ⊘ The tag was already validated by `Request::decode` (`ring_provenance_tag` refuses an
/// unknown one), so this match is total by construction and the `_` arm cannot be reached by
/// a frame. It answers `StoreSlice` there rather than `OwnObject` because the two arms are
/// asymmetric: `StoreSlice` names no handle and the adapter refuses it unless it was expected,
/// while `OwnObject` would fabricate a `HostHandle` out of whatever `a` happened to be.
fn ring_provenance(kind: u8, a: u64, b: u64) -> kayfabe_isolate::RingProvenance {
    if kind == crate::proto::RING_PROVENANCE_OWN_OBJECT {
        kayfabe_isolate::RingProvenance::OwnObject(raw(a))
    } else {
        kayfabe_isolate::RingProvenance::StoreSlice { offset: a, len: b }
    }
}

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
        // ★★★ The named boundary of decision (b) keeps its own wire form all the way
        // out. Collapsed into `Other`, the VMM could not tell *"the bytes are on the
        // card"* from *"the host refused"*, and the whole value of naming the boundary
        // is that those are different facts with different consequences.
        RmError::NotExportableAsMemory { memory } => WireError::NotExportableAsMemory(memory.raw()),
        // ★★★ Likewise its own wire form: *"this VM was not launched with a shared memory
        // backing"* is a DEPLOYMENT fact and *"the host refused"* is not, and the two have
        // different fixes.
        RmError::GuestRamUnavailable => WireError::GuestRamUnavailable,
        // #102: a child backend cannot produce `PlacementRefused` — the check that mints
        // it lives in the PARENT's `Worker::execute`, above the wire. Listed explicitly
        // rather than caught by a wildcard, so adding a variant stays a compile error.
        // ★ w734: `ViewNotReleasable` joins them for the same reason. It is minted in the
        // PARENT's `Worker::release_device_view`, above the wire — a child never sees a
        // `DeviceView` and so can never produce it. Listed rather than wildcarded, so the
        // next variant is a compile error here too.
        RmError::Wedged
        | RmError::ForeignHandle { .. }
        | RmError::PlacementRefused { .. }
        | RmError::ViewNotReleasable => WireError::Other(crate::rm::NOT_ON_THIS_RUNG),
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
    /// A `/dev/null` handle — a character device, which is what a birth-client descriptor
    /// must be. ⊘ Not a `memfd`: the check is against the KERNEL, so the test has to hand it
    /// something the kernel really classifies that way.
    fn a_char_device() -> OwnedFd {
        // ⊘ `File -> OwnedFd` is the SAFE conversion; this crate forbids `unsafe` and a test
        // is not an exemption from that.
        OwnedFd::from(std::fs::File::open("/dev/null").expect("/dev/null"))
    }

    fn a_regular_file() -> OwnedFd {
        let p = std::env::temp_dir().join(format!("kf-w753-birth-{}", std::process::id()));
        let f = std::fs::File::create(&p).expect("temp file");
        let _ = std::fs::remove_file(&p);
        OwnedFd::from(f)
    }

    fn loopback() -> LoopbackRm {
        let shared = LoopbackShared::new(ParkVerb::Nothing, None).expect("pipe");
        LoopbackRm::new(
            IsolateId::new(1, GpuId(0)),
            shared,
            Arc::new(ChildExports::new()),
        )
        .expect("dup")
    }

    /// The scratchpad's own id — the only isolate a birth client may reach.
    fn scratchpad() -> IsolateId {
        IsolateId::new(crate::SCRATCHPAD_ISOLATE_PROC, GpuId(0))
    }

    /// ★★★★★ **CONSTRAINT 32's KNOWN-POSITIVE FOR THE fd-IN PATH — the one that catches a
    /// reader with no control buffer.**
    ///
    /// ⊘⊘ **Not a redundant argument check.** A `recvmsg` without a control buffer does not
    /// refuse a descriptor: the kernel **closes it and delivers the body perfectly**. So the
    /// *only* observable of that bug, anywhere in the system, is an `AdoptBirthClient`
    /// reaching this function with an empty `fds`. Answered `Ok`, the VMM would record a
    /// hand-over, the scratchpad would hold a client handle naming a session it cannot
    /// reach, and the first evidence would be an RM refusal hundreds of ioctls later in a
    /// different subsystem.
    ///
    /// ⇒ refused with a code of **its own**, so *"the transport dropped them"* can never be
    /// read as *"RM said no"*.
    #[test]
    fn a_birth_client_without_its_descriptors_is_refused_by_its_own_name() {
        let mut rm = loopback();
        let (reply, carried) = serve_one(
            &mut rm,
            Request::AdoptBirthClient {
                client: 0xc1d0_0001,
                minted_by_proc: 2,
            },
            &ChildExports::new(),
            Vec::new(),
            scratchpad(),
        );
        assert_eq!(
            reply,
            Reply::Failed(WireError::Other(crate::rm::BIRTH_CLIENT_NO_DESCRIPTORS)),
            "★★★ a birth client with no descriptors must be refused by ITS OWN name — a \
             generic refusal makes a transport bug indistinguishable from RM's answer"
        );
        assert!(carried.is_none());
    }

    /// ⊘ **A null `hRoot` is the one handle RM interprets rather than refuses.** Both
    /// descriptors are present so this cannot pass for the previous test's reason.
    #[test]
    fn a_birth_client_named_zero_is_refused_even_with_both_descriptors() {
        let mut rm = loopback();
        let (reply, _) = serve_one(
            &mut rm,
            Request::AdoptBirthClient {
                client: 0,
                minted_by_proc: 2,
            },
            &ChildExports::new(),
            vec![a_char_device(), a_char_device()],
            scratchpad(),
        );
        assert_eq!(
            reply,
            Reply::Failed(WireError::Other(crate::rm::BIRTH_CLIENT_NULL_HANDLE)),
        );
    }

    /// ★★★ **The kind check is against the KERNEL, not the sender's claim** — and the
    /// SECOND descriptor is checked too. A check that only looks at the first is a check on
    /// half the message.
    #[test]
    fn a_birth_client_descriptor_that_is_not_a_char_device_is_refused() {
        let mut rm = loopback();
        let (reply, _) = serve_one(
            &mut rm,
            Request::AdoptBirthClient {
                client: 0xc1d0_0001,
                minted_by_proc: 2,
            },
            &ChildExports::new(),
            vec![a_char_device(), a_regular_file()],
            scratchpad(),
        );
        assert_eq!(
            reply,
            Reply::Failed(WireError::Other(crate::rm::BIRTH_CLIENT_NOT_A_CHAR_DEVICE)),
        );
    }

    /// ★★★★★ **EXACTLY ONE REQUEST MAY CARRY DESCRIPTORS**, and every other is refused by
    /// name with whatever arrived **closed**.
    ///
    /// ⊘ The reader's allowance is a constant, because one reader serves every verb and has
    /// not decoded the body when it sizes the control buffer. ⇒ without this, a peer could
    /// attach a descriptor to *any* request and the child would take it — the rule would
    /// exist in a doc comment and nowhere else.
    #[test]
    fn a_descriptor_on_a_request_that_may_not_carry_one_is_refused() {
        let mut rm = loopback();
        let (reply, carried) = serve_one(
            &mut rm,
            Request::AllocVaSpaceBare,
            &ChildExports::new(),
            vec![a_char_device()],
            scratchpad(),
        );
        assert_eq!(
            reply,
            Reply::Failed(WireError::Other(crate::rm::FD_ON_A_BYTES_ONLY_REQUEST)),
        );
        assert!(carried.is_none());
    }

    /// ⊘ **NON-VACUITY for the four tests above**: the same verb, well formed, gets past
    /// every gate — so those refusals are about what they say they are about, and not about
    /// `serve_one` refusing this verb unconditionally.
    ///
    /// ⊘ Asserted as *"none of the four"* rather than as one value: pinning the backend's
    /// own answer here would make this test fail the day the real backend lands, which is
    /// precisely the day it should still pass.
    #[test]
    fn a_well_formed_birth_client_reaches_the_backend() {
        let mut rm = loopback();
        let (reply, _) = serve_one(
            &mut rm,
            Request::AdoptBirthClient {
                client: 0xc1d0_0001,
                minted_by_proc: 2,
            },
            &ChildExports::new(),
            vec![a_char_device(), a_char_device()],
            scratchpad(),
        );
        for (name, code) in [
            (
                "BIRTH_CLIENT_NO_DESCRIPTORS",
                crate::rm::BIRTH_CLIENT_NO_DESCRIPTORS,
            ),
            (
                "BIRTH_CLIENT_NOT_A_CHAR_DEVICE",
                crate::rm::BIRTH_CLIENT_NOT_A_CHAR_DEVICE,
            ),
            ("BIRTH_CLIENT_NULL_HANDLE", crate::rm::BIRTH_CLIENT_NULL_HANDLE),
            (
                "FD_ON_A_BYTES_ONLY_REQUEST",
                crate::rm::FD_ON_A_BYTES_ONLY_REQUEST,
            ),
        ] {
            assert_ne!(
                reply,
                Reply::Failed(WireError::Other(code)),
                "★★★ VACUITY — a well-formed birth client was refused by {name}, so the \
                 refusal tests above may be passing for a reason unrelated to what they claim"
            );
        }
    }

    #[test]
    fn an_oversized_fabricated_aperture_read_is_refused_before_anything_is_allocated() {
        let shared = LoopbackShared::new(ParkVerb::Nothing, None).expect("pipe");
        let mut rm = LoopbackRm::new(
            IsolateId::new(1, GpuId(0)),
            shared,
            Arc::new(ChildExports::new()),
        )
        .expect("dup");

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
                bare_vaspaces: false,
                proc: 3,
                gpu: 1,
                workers: 4,
                rm: RmMode::Loopback,
                park: ParkVerb::Nothing,
                // ⊘ Absent on the command line is `false`, and the sample says so rather than
                // leaving the default untested — the flag decides whether a process brings
                // CUDA up before it is sandboxed.
                cuda_walk: false,
                guest_ram_bytes: 0,
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
