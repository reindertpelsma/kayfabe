//! ★ The transport's fixture — **not** a model of RM.
//!
//! `host_execution_plane.md` §5 warns that with the host side entirely mocked, *"some of it
//! is machinery validated by a fixture written to satisfy it"*. So this file states what it
//! is for and, more importantly, what it refuses to be.
//!
//! ## What it exists to make testable
//!
//! Everything about the isolate that is **not** the driver: a real child process, a real
//! socket, a real descriptor table, a real thread per worker, real blocking, and — the one
//! that no in-process double can give — **real signal-driven cancellation**. A parked verb
//! here is a thread inside `read(2)` on a pipe, so a real `SIGUSR1` with no `SA_RESTART`
//! really returns `EINTR`, through the real handler, into the real error classification,
//! back over the real socket. That path is the entire §7.2 handshake and it is untestable
//! against a condvar.
//!
//! ## The ONE RM semantic it models, and why only one
//!
//! **Per-client serialisation.** Every verb takes one lock for its whole duration, because
//! that is what RM does (`rm_concurrency_semantics`, C-measured; the write lock at
//! `ogkm-610:`/`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_server.c:778`). A fixture
//! that answered concurrently would make the pool look like it buys wire concurrency, which
//! is the belief `host_execution_plane.md` §2.0 found twelve tests resting on.
//!
//! Nothing else is modelled. Handles are a counter, allocations are set insertions, and
//! there is no attempt at RM's object model. **That is deliberate**: a fixture that grows
//! toward being a driver becomes the thing the design gets validated against, and the whole
//! point of #91 is to stop doing that.
//!
//! ## The lock is NOT registered with the leaf witness, and that is not an oversight
//!
//! `kayfabe_util::leafwitness` exists to catch *our* adapter holding *our* lock across a
//! blocking call. This lock is standing in for one inside the kernel. Registering it would
//! make every parked verb trip an assert about a rule it is not breaking — and would train
//! the next reader to suppress the witness, which is the expensive mistake.

use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuVa};
use kayfabe_isolate::{HostHandle, IsolateId, RmBackend, RmError};
use kayfabe_vmm::SurfaceHandle;
use std::collections::BTreeSet;
use std::io::Read;
use std::sync::{Arc, Mutex};

/// Which verb parks forever. One knob, chosen because the hazard the design is about is a
/// verb that **does not come back**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkVerb {
    /// Nothing parks.
    Nothing,
    /// [`RmBackend::alloc_sysmem`] parks. Chosen because it is the one verb with no
    /// argument the parent has to construct from a previous reply, so a test can reach it
    /// in one call.
    Sysmem,
}

impl ParkVerb {
    /// Parse the child's `--park` argument.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(ParkVerb::Nothing),
            "sysmem" => Some(ParkVerb::Sysmem),
            _ => None,
        }
    }

    /// The argument form.
    #[must_use]
    pub fn as_arg(self) -> &'static str {
        match self {
            ParkVerb::Nothing => "none",
            ParkVerb::Sysmem => "sysmem",
        }
    }
}

#[derive(Debug, Default)]
struct Table {
    handles: BTreeSet<u64>,
    next: u64,
}

/// The isolate-wide state every worker's backend shares — **one client, one lock**.
#[derive(Debug)]
pub struct LoopbackShared {
    client: Mutex<Table>,
    park_on: ParkVerb,
    /// Held for the process's life so a park's `read` blocks instead of seeing end of
    /// stream. Dropping it is how the child would release every parked verb at once, which
    /// is deliberately not exposed: the hazard is a verb that never returns.
    _park_writer: std::io::PipeWriter,
    park_reader: std::io::PipeReader,
}

impl LoopbackShared {
    /// Create the shared state.
    ///
    /// # Errors
    /// The `pipe(2)` failed.
    pub fn new(park_on: ParkVerb) -> std::io::Result<Arc<Self>> {
        let (park_reader, _park_writer) = std::io::pipe()?;
        Ok(Arc::new(LoopbackShared {
            client: Mutex::new(Table {
                handles: BTreeSet::new(),
                // ★ The same first value in every isolate, exactly as
                // `kayfabe_isolate_host::rm` mints and exactly as RM does from one
                // `RS_CLIENT_HANDLE_BASE`: two isolates' n-th handles collide.
                next: 0x00CA_FE01,
            }),
            park_on,
            _park_writer,
            park_reader,
        }))
    }
}

/// One worker's loopback backend.
#[derive(Debug)]
pub struct LoopbackRm {
    id: IsolateId,
    shared: Arc<LoopbackShared>,
    /// This worker's own reader for the park pipe. Per-worker so a park is this thread's
    /// blocking call and not a contended one.
    park: std::io::PipeReader,
}

impl LoopbackRm {
    /// One worker's backend over `shared`.
    ///
    /// # Errors
    /// The descriptor could not be duplicated.
    pub fn new(id: IsolateId, shared: Arc<LoopbackShared>) -> std::io::Result<Self> {
        let park = shared.park_reader.try_clone()?;
        Ok(LoopbackRm { id, shared, park })
    }

    /// Take the client lock, park if this verb is the parking one, and mint. The lock is
    /// held **across** the park, which is the RM semantic being modelled.
    fn verb(&mut self, parks: bool) -> Result<u64, RmError> {
        let mut table = self.shared.client.lock().unwrap_or_else(|e| e.into_inner());
        if parks && self.shared.park_on == ParkVerb::Sysmem {
            let mut byte = [0u8; 1];
            match self.park.read(&mut byte) {
                // Only a signal ends a park. The `Ok` arms are unreachable while the
                // writer is alive and are treated as a release rather than a surprise.
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    return Err(RmError::Interrupted);
                }
                Err(_) | Ok(_) => {}
            }
        }
        let h = table.next;
        table.next += 1;
        table.handles.insert(h);
        Ok(h)
    }

    fn known(&self, h: HostHandle) -> Result<(), RmError> {
        let table = self.shared.client.lock().unwrap_or_else(|e| e.into_inner());
        if table.handles.contains(&h.raw()) {
            Ok(())
        } else {
            Err(RmError::BadHandle(h))
        }
    }

    fn stamp(&self, raw: u64) -> HostHandle {
        HostHandle::new(self.id, raw)
    }
}

impl RmBackend for LoopbackRm {
    fn alloc(
        &mut self,
        parent: HostHandle,
        _class: ClassId,
        _params: &[u8],
    ) -> Result<HostHandle, RmError> {
        if parent != HostHandle::NULL {
            self.known(parent)?;
        }
        let h = self.verb(false)?;
        Ok(self.stamp(h))
    }

    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError> {
        let h = self.verb(false)?;
        Ok(self.stamp(h))
    }

    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        if len == 0 {
            return Err(RmError::NoMemory);
        }
        let h = self.verb(true)?;
        Ok(self.stamp(h))
    }

    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        _engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError> {
        self.known(vas)?;
        let h = self.verb(false)?;
        Ok((self.stamp(h), h | 0x1_0000_0000))
    }

    fn alloc_engine_object(
        &mut self,
        chan: HostHandle,
        _class: ClassId,
        _params: &[u8],
    ) -> Result<HostHandle, RmError> {
        self.known(chan)?;
        let h = self.verb(false)?;
        Ok(self.stamp(h))
    }

    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError> {
        self.known(chan)?;
        self.verb(false)?;
        Ok(())
    }

    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        self.known(obj)?;
        let mut table = self.shared.client.lock().unwrap_or_else(|e| e.into_inner());
        table.handles.remove(&obj.raw());
        Ok(())
    }

    fn control(
        &mut self,
        obj: HostHandle,
        _cmd: ControlCmd,
        _payload: &mut [u8],
    ) -> Result<(), RmError> {
        self.known(obj)?;
        self.verb(false)?;
        Ok(())
    }

    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        _len: u64,
        at: GpuVa,
    ) -> Result<u64, RmError> {
        self.known(vas)?;
        self.known(memory)?;
        let _ = self.verb(false)?;
        // #102 address identity: a placement request is honoured, not invented. The
        // loopback double used to hand back a handle-derived VA, which under the new
        // contract is precisely the "backend silently chose" failure `PlacementRefused`
        // exists to catch — so it would fail every publish for a reason that has nothing
        // to do with what this double is for.
        Ok(at.0)
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, _gpu_va: u64) -> Result<(), RmError> {
        self.known(vas)?;
        self.verb(false)?;
        Ok(())
    }

    fn ring_doorbell(&mut self, _host_token: u64) -> Result<(), RmError> {
        self.verb(false)?;
        Ok(())
    }

    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        self.known(memory)?;
        let h = self.verb(false)?;
        Ok(SurfaceHandle(h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kayfabe_arch::ids::GpuId;

    fn rm(park: ParkVerb) -> LoopbackRm {
        let shared = LoopbackShared::new(park).expect("pipe");
        LoopbackRm::new(IsolateId::new(1, GpuId(0)), shared).expect("dup")
    }

    #[test]
    fn an_unknown_park_verb_is_refused_rather_than_defaulted() {
        assert_eq!(ParkVerb::parse("none"), Some(ParkVerb::Nothing));
        assert_eq!(ParkVerb::parse("sysmem"), Some(ParkVerb::Sysmem));
        assert_eq!(ParkVerb::parse("Sysmem"), None);
        assert_eq!(ParkVerb::parse(""), None);
    }

    #[test]
    fn park_verbs_round_trip_through_their_argument_form() {
        for v in [ParkVerb::Nothing, ParkVerb::Sysmem] {
            assert_eq!(ParkVerb::parse(v.as_arg()), Some(v));
        }
    }

    /// ★ Two isolates' n-th handles genuinely collide — the property a real host has and
    /// the mock had to be taught. Asserted between two INDEPENDENT shared states, which is
    /// what two child processes are.
    #[test]
    fn two_isolates_mint_the_same_raw_values() {
        let mut a = rm(ParkVerb::Nothing);
        let mut b = LoopbackRm::new(
            IsolateId::new(2, GpuId(0)),
            LoopbackShared::new(ParkVerb::Nothing).expect("pipe"),
        )
        .expect("dup");
        let ha = a.alloc_vaspace().expect("a");
        let hb = b.alloc_vaspace().expect("b");
        assert_eq!(ha.raw(), hb.raw(), "the raw values must collide");
        assert_ne!(ha.isolate(), hb.isolate(), "…and the namespaces must not");
        assert_ne!(ha, hb, "so the handles are still distinguishable to US");
    }

    #[test]
    fn an_unknown_handle_is_a_bad_handle_with_the_value_presented() {
        let mut r = rm(ParkVerb::Nothing);
        let bogus = HostHandle::new(IsolateId::new(1, GpuId(0)), 0xDEAD);
        assert_eq!(r.free(bogus), Err(RmError::BadHandle(bogus)));
    }

    #[test]
    fn a_freed_handle_stops_being_known() {
        let mut r = rm(ParkVerb::Nothing);
        let h = r.alloc_vaspace().expect("alloc");
        assert_eq!(r.free(h), Ok(()));
        assert_eq!(r.free(h), Err(RmError::BadHandle(h)));
    }

    #[test]
    fn a_zero_length_sysmem_request_is_refused() {
        let mut r = rm(ParkVerb::Nothing);
        assert_eq!(r.alloc_sysmem(0), Err(RmError::NoMemory));
    }

    /// ★★ The park is a REAL blocking `read(2)`, released by a REAL signal — the property
    /// no in-process double can provide. Non-vacuity: the same verb returns immediately
    /// when parking is off.
    #[test]
    fn a_parked_verb_blocks_until_a_break_signal_and_reports_interrupted() {
        kayfabe_linux_raw::install_break_handler().expect("handler");
        let shared = LoopbackShared::new(ParkVerb::Sysmem).expect("pipe");
        let mut worker =
            LoopbackRm::new(IsolateId::new(1, GpuId(0)), Arc::clone(&shared)).expect("dup");
        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let t = std::thread::spawn(move || {
            tid_tx
                .send(kayfabe_linux_raw::current_thread_id())
                .expect("tid");
            done_tx.send(worker.alloc_sysmem(0x1000)).expect("result");
        });
        let tid = tid_rx.recv().expect("tid");
        let result = loop {
            assert!(kayfabe_linux_raw::interrupt_thread(tid).expect("tgkill"));
            if let Ok(r) = done_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                break r;
            }
        };
        assert_eq!(result, Err(RmError::Interrupted));
        t.join().expect("join");

        // Non-vacuity: with parking off the same verb returns without any signal at all.
        let mut quiet = rm(ParkVerb::Nothing);
        assert!(quiet.alloc_sysmem(0x1000).is_ok());
    }

    /// ★★ The one RM semantic this fixture models: a parked verb holds the client lock, so
    /// a **sibling worker of the same isolate** cannot proceed. Asserted as a progress
    /// edge, not a sleep — the sibling's completion channel stays empty while the park is
    /// held and delivers once it is released.
    #[test]
    fn a_parked_verb_holds_the_client_lock_against_its_own_siblings() {
        kayfabe_linux_raw::install_break_handler().expect("handler");
        let shared = LoopbackShared::new(ParkVerb::Sysmem).expect("pipe");
        let id = IsolateId::new(1, GpuId(0));
        let mut parker = LoopbackRm::new(id, Arc::clone(&shared)).expect("dup");
        let mut sibling = LoopbackRm::new(id, Arc::clone(&shared)).expect("dup");

        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        let (parked_tx, parked_rx) = std::sync::mpsc::channel();
        let a = std::thread::spawn(move || {
            tid_tx
                .send(kayfabe_linux_raw::current_thread_id())
                .expect("tid");
            parked_tx.send(parker.alloc_sysmem(0x1000)).expect("send");
        });
        let tid = tid_rx.recv().expect("tid");

        // The sibling issues a NON-parking verb. It must still be blocked, because the lock
        // is per client and not per verb.
        let (sib_tx, sib_rx) = std::sync::mpsc::channel();
        let b = std::thread::spawn(move || {
            sib_tx.send(sibling.alloc_vaspace()).expect("send");
        });

        // Deliver the break until the parked verb reports it. Every attempt is also a
        // sample of the sibling's channel: it must be empty every time.
        let parked_result = loop {
            assert!(kayfabe_linux_raw::interrupt_thread(tid).expect("tgkill"));
            match parked_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(r) => break r,
                Err(_) => assert!(
                    sib_rx.try_recv().is_err(),
                    "a sibling on the SAME client made progress while a verb was parked"
                ),
            }
        };
        assert_eq!(parked_result, Err(RmError::Interrupted));
        // And once released, the sibling completes.
        assert!(sib_rx.recv().expect("sibling").is_ok());
        a.join().expect("join a");
        b.join().expect("join b");
    }
}
