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

use crate::export::ChildExports;
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuVa};
use kayfabe_isolate::{
    CeSubCopy, ExportRequest, ExportSource, ExportedBacking, FbLeafJoined, GuestRamGrant,
    GuestRamMapped, HostHandle, HostedObject, IsolateId, RmBackend, RmError,
};
use kayfabe_vmm::SurfaceHandle;
use std::collections::BTreeSet;
use std::io::{Read, Write};
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
    /// ★★★ The **progress edge**: written once, immediately before a park's blocking read.
    ///
    /// A parent that wants to act on a parked verb needs to know the verb is *parked* — not
    /// merely that no reply has arrived yet. Those are different facts, and the second is
    /// strictly weaker: "no reply yet" is also true when the chain has not started. Waiting
    /// on a duration to tell them apart is a **bet on a host round trip**, and
    /// `abandon_releases_a_wedged_requester_with_wedged` lost that bet ~0.5 % of the time
    /// (deterministically at 0 ms — see `real_isolate.rs`).
    ///
    /// ⊘ This is **test-support scaffolding, not protocol**. It exists only because the park
    /// itself is induced by the fixture (`--park`), and the component that chooses to park is
    /// the one honest place to announce it. It is `None` in every isolate that does not park,
    /// so a production isolate has neither the fd nor the write.
    park_witness: Option<std::io::PipeWriter>,
}

impl LoopbackShared {
    /// Create the shared state.
    ///
    /// # Errors
    /// The `pipe(2)` failed.
    pub fn new(
        park_on: ParkVerb,
        park_witness: Option<std::io::PipeWriter>,
    ) -> std::io::Result<Arc<Self>> {
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
            park_witness,
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
    /// ★ The isolate's export table (`crate::export`), shared with every sibling worker.
    exports: Arc<ChildExports>,
    /// This isolate's guest-RAM plane, or `None`. ⊘ The fixture uses the **real** plane, not
    /// a model of one: `mmap`ping a `memfd` needs no GPU, so faking it here would be a
    /// second implementation of the one thing this fixture could have honestly run.
    guest_ram: Option<Arc<crate::guestram::GuestRamPlane>>,
    /// ★★★★★ This isolate's joined framebuffer leaves, shared with every sibling worker.
    /// ⊘ The **real** table for the same reason `guest_ram` is real: mapping a `memfd` and
    /// remembering which framebuffer range it stands for needs no GPU, so a model of it here
    /// would be a second implementation of the one half this fixture can honestly run.
    fb_joins: Option<Arc<crate::fbjoin::FbJoinTable>>,
}

impl LoopbackRm {
    /// One worker's backend over `shared`.
    ///
    /// # Errors
    /// The descriptor could not be duplicated.
    pub fn new(
        id: IsolateId,
        shared: Arc<LoopbackShared>,
        exports: Arc<ChildExports>,
    ) -> std::io::Result<Self> {
        let park = shared.park_reader.try_clone()?;
        Ok(LoopbackRm {
            id,
            shared,
            park,
            exports,
            guest_ram: None,
            fb_joins: None,
        })
    }

    /// Install this isolate's guest-RAM plane (or, with `None`, state that it has none).
    #[must_use]
    pub fn with_guest_ram(mut self, plane: Option<Arc<crate::guestram::GuestRamPlane>>) -> Self {
        self.guest_ram = plane;
        self
    }

    /// Install this isolate's shared framebuffer-join table. See `crate::fbjoin`.
    #[must_use]
    pub fn with_fb_joins(mut self, joins: Arc<crate::fbjoin::FbJoinTable>) -> Self {
        self.fb_joins = Some(joins);
        self
    }

    /// Take the client lock, park if this verb is the parking one, and mint. The lock is
    /// held **across** the park, which is the RM semantic being modelled.
    fn verb(&mut self, parks: bool) -> Result<u64, RmError> {
        let mut table = self.shared.client.lock().unwrap_or_else(|e| e.into_inner());
        if parks && self.shared.park_on == ParkVerb::Sysmem {
            // ★ Announce BEFORE blocking, and while holding the client lock. When the parent
            // observes this byte, every earlier verb in the chain has already returned — that
            // is precisely the fact a duration cannot establish. A failed write is deliberately
            // ignored: the witness is an observation aid, and a parent that stopped listening
            // must not change whether this verb parks.
            if let Some(w) = self.shared.park_witness.as_ref() {
                let _ = (&mut &*w).write_all(b"P");
            }
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

    // ⊘ `self.verb(false)` — this loopback does NOT park on a vidmem alloc, and that is a
    // decision rather than an omission. [`ParkVerb`] names `Sysmem` as the parking verb
    // because it is the one verb with no argument built from a previous reply; vidmem has
    // the same property, so adding a second parking verb would give the wedge tests two
    // ways to express one scenario and no reason to prefer either.
    fn alloc_vidmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        if len == 0 {
            return Err(RmError::NoMemory);
        }
        let h = self.verb(false)?;
        Ok(self.stamp(h))
    }

    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        _engine: EngineKind,
        hosting: Option<HostedObject<'_>>,
        adopt: Option<kayfabe_isolate::AdoptedGuestRing>,
    ) -> Result<(HostHandle, u64), RmError> {
        // ★★★★★ **A SILENT DISCARD, MADE LOUD.** This arm took `_adopt` and dropped it. A boot
        // whose plane selector picked the loopback therefore produced a channel born over
        // nothing at all, on a log **indistinguishable** from a real isolate that adopted
        // nothing — exactly the pair `w261` could not separate, one layer lower.
        //
        // ⊘ It is still a discard, and deliberately so: this backend has no RM and no joined
        // framebuffer leaf, so there is nothing here that *could* honour a ring. What changes
        // is that the discard is now on disk. ⇒ *"no `GR-BIRTH` line at all"* stops being
        // reachable through the plane selector, which is what makes a zero in the armed
        // census a **measured** zero (`a_census_zero_needs_a_known_positive`).
        let offer = crate::rm::BirthOffer::read(hosting.is_some(), adopt.is_some());
        let userd_offer = crate::rm::BirthOffer::read(
            hosting.is_some(),
            adopt.is_some_and(|a| a.userd.is_some()),
        );
        eprintln!(
            "kayfabe-isolate: GR-BIRTH vas={:#x} adopt={} userd={} ⊘ LOOPBACK BACKEND — \
             DISCARDED. This plane has no RM and no joined leaf. ⚠ This boot's channels are \
             NOT host channels, so nothing in it measures leg A or leg B",
            vas.raw(),
            offer.as_str(crate::rm::BirthLimb::Ring),
            userd_offer.as_str(crate::rm::BirthLimb::Userd),
        );
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

    fn ce_copy(&mut self, vas: HostHandle, _sub: CeSubCopy) -> Result<(), RmError> {
        self.known(vas)?;
        self.verb(false)?;
        Ok(())
    }

    /// ★ **This fixture models no device memory, and says so.**
    ///
    /// `Ok(false)` is the honest answer for an isolate whose fabricated aperture maps
    /// nothing — and it is the *correct* one, not a stub: the walker turns it into a loud
    /// `MISS = FAULT`, which is exactly what should happen when the content source has no
    /// content. Growing a byte store here is the thing this file's own header refuses
    /// ("a fixture that grows toward being a driver becomes the thing the design gets
    /// validated against"); the byte-level model lives in `kayfabe_mocks::MockRmBackend`,
    /// beside the copies that write it.
    ///
    /// What it still proves, and is the reason it is not `Err`: the verb crosses the real
    /// socket, into the real child process, and back.
    fn fb_read(&mut self, _phys: u64, _buf: &mut [u8]) -> Result<bool, RmError> {
        self.verb(false)?;
        Ok(false)
    }

    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        self.known(memory)?;
        let h = self.verb(false)?;
        Ok(SurfaceHandle(h))
    }

    /// ★★★ Decision (b), through the fixture — and it is **real**, not modelled.
    ///
    /// This is the one verb where the fixture and the production backend do the *identical
    /// thing*: [`crate::rm::mint_fabricated`] is called by both. That is not a fixture
    /// growing toward being a driver (the thing this file's header refuses) — minting a
    /// `memfd` is not an RM semantic, and there is nothing here to model. It means the
    /// crossing can be exercised end to end, through a real child process and a real
    /// socket, on a box with **no GPU**.
    ///
    /// ⊘ The device arm refuses here for the same reason and with the same error as the
    /// real backend. Answering it with a `memfd` — bytes we invented in place of the
    /// card's — would be exactly the fixture-shaped lie that makes a boundary look
    /// crossed when it is not.
    fn export_backing(&mut self, want: ExportRequest) -> Result<ExportedBacking, RmError> {
        if let ExportSource::HostDeviceMemory { memory } = want.source {
            self.known(memory)?;
            return Err(RmError::NotExportableAsMemory { memory });
        }
        // ★ Under the client lock, like every other verb: a backing is minted by the same
        // serialised path RM would serialise, so a parked sibling blocks this too.
        self.verb(false)?;
        crate::rm::mint_fabricated(&self.exports, want)
    }

    /// ★★★★★ The join, through the fixture — **half real, and the half that is modelled is
    /// named**.
    ///
    /// Real: the `memfd`, the isolate's `mmap` of it, the join table entry, and therefore the
    /// whole of the two-views property. That half needs no GPU, so it is not modelled and the
    /// crossing can be exercised end to end through a real child on a box with no card.
    ///
    /// ⊘ **Modelled: the RM half.** There is no `OS_DESCRIPTOR` and no GPU MMU here, so
    /// `memory` is a fixture handle and `host_va` is `at` **by fiat**. ⇒ A green line from
    /// this backend says the VMM's plumbing works; it says **nothing** about whether RM would
    /// place the mapping, which is exactly what `fb_cpu_view.md` §3 had to measure on real
    /// hardware. Reading it as more is the fixture-shaped lie this file's header refuses.
    fn join_fb_leaf(
        &mut self,
        vas: HostHandle,
        len: u64,
        at: GpuVa,
        phys: u64,
    ) -> Result<FbLeafJoined, RmError> {
        // ★ The pool gate FIRST, in the same order `HostRmBackend::join_fb_leaf` checks it.
        // ⊘ Deliberately before the handle check: a backend with no shared table cannot serve
        // this verb for ANY argument, and answering `BadHandle` would name the caller for a
        // fault that is entirely ours.
        let table = Arc::clone(
            self.fb_joins
                .as_ref()
                .ok_or(RmError::Other(crate::rm::FB_JOIN_NO_TABLE))?,
        );
        self.known(vas)?;
        let h = self.verb(false)?;
        let backing = crate::rm::mint_fabricated(
            &self.exports,
            ExportRequest {
                source: ExportSource::Fabricated,
                len,
                prot: kayfabe_vmm::Prot::ReadWrite,
            },
        )?;
        let fd = self
            .exports
            .lend(backing.token)
            .map_err(|_| RmError::NoMemory)?;
        let region = kayfabe_linux_raw::MappedRegion::map(
            kayfabe_linux_raw::Backing::SharedFile {
                fd: std::os::fd::AsFd::as_fd(&fd),
                offset: 0,
            },
            len,
            kayfabe_linux_raw::HostProt::ReadWrite,
            kayfabe_linux_raw::CachePolicy::WriteBack,
            kayfabe_linux_raw::HostPageSize::query(),
        )
        .map_err(|_| RmError::NoMemory)?;
        drop(fd);
        table.install(phys, len, at.0, region);
        Ok(FbLeafJoined {
            backing,
            memory: HostHandle::new(self.id, h),
            host_va: at.0,
        })
    }

    /// The instrument, through the fixture — and here it is **entirely** real: it reads and
    /// writes the same `mmap` the production backend describes to RM.
    fn fb_join_peek(
        &mut self,
        phys: u64,
        buf: &mut [u8],
        poke: Option<u32>,
    ) -> Result<bool, RmError> {
        let table = self
            .fb_joins
            .as_ref()
            .ok_or(RmError::Other(crate::rm::FB_JOIN_NO_TABLE))?;
        table.peek(phys, buf, poke).map_err(|_| RmError::NoMemory)
    }

    fn map_guest_ram(&mut self, grant: GuestRamGrant) -> Result<GuestRamMapped, RmError> {
        let Some(plane) = self.guest_ram.clone() else {
            return Err(RmError::GuestRamUnavailable);
        };
        // Under the client lock, like every other verb — RM serialises per client and the
        // fixture's whole job is to model that, not to be fast.
        self.verb(false)?;
        let raw = plane.honour(grant)?;
        Ok(GuestRamMapped {
            region: HostHandle::new(self.id, raw),
            len: grant.len(),
        })
    }

    fn unmap_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<(), RmError> {
        let Some(plane) = self.guest_ram.clone() else {
            return Err(RmError::GuestRamUnavailable);
        };
        self.verb(false)?;
        plane.release(mapped.region.raw())
    }

    /// ★★ The fixture's arm, and it is honest about **which half it models**.
    ///
    /// It checks the one thing that does not need a GPU — that the name really is a live
    /// guest-RAM mapping *this* isolate made — and then mints an object handle from the
    /// same table every other alloc here uses. ⊘ **No `OS_DESCRIPTOR` is issued and none
    /// is modelled**: this double has no RM, so the only fact it can carry forward is that
    /// the *chain* is wired, which is exactly what `tests/guest_ram.rs` uses it for on a
    /// box with no card.
    ///
    /// ⊘ Do not read a green loopback run as evidence that RM accepted anything. The
    /// verb's real content — that the driver will `pin_user_pages`-walk a host VA it was
    /// handed and build a memory object over guest RAM — is unmodellable here and is
    /// measured only on hardware.
    fn describe_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<HostHandle, RmError> {
        let Some(plane) = self.guest_ram.clone() else {
            return Err(RmError::GuestRamUnavailable);
        };
        // ★ The liveness check is real, and it is the reason this is not simply
        // `alloc_sysmem` under another name: a name that no longer maps anything must be
        // refused here, or the fixture would happily "describe" a released window.
        plane.with_region(mapped.region.raw(), |_| ())?;
        let h = self.verb(false)?;
        Ok(self.stamp(h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kayfabe_arch::ids::GpuId;

    /// A fresh export table per fixture backend. Its own, deliberately: these tests are
    /// about the client lock and handle namespaces, and a shared table would silently
    /// couple two "independent" isolates through a resource neither test names.
    fn exports() -> Arc<ChildExports> {
        Arc::new(ChildExports::new())
    }

    fn rm(park: ParkVerb) -> LoopbackRm {
        let shared = LoopbackShared::new(park, None).expect("pipe");
        LoopbackRm::new(IsolateId::new(1, GpuId(0)), shared, exports()).expect("dup")
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
            LoopbackShared::new(ParkVerb::Nothing, None).expect("pipe"),
            exports(),
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
        let shared = LoopbackShared::new(ParkVerb::Sysmem, None).expect("pipe");
        let mut worker =
            LoopbackRm::new(IsolateId::new(1, GpuId(0)), Arc::clone(&shared), exports())
                .expect("dup");
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
    /// edge, not a sleep — the sibling's completion channel is empty while the park is held
    /// and delivers once it is released.
    ///
    /// ## ★ The interval is ESTABLISHED, not assumed — and why that is the whole test
    ///
    /// The obvious shape — publish the parker's tid, spawn the sibling, then re-sample the
    /// sibling's channel every time the parked verb has not answered yet — failed under
    /// load and was set aside as a flake. It is neither a flake nor a product bug: it
    /// asserts *"the sibling made no progress"* across two intervals in which that is
    /// **not the property being tested**, and both were reproduced at 20/20 by widening
    /// them with a sleep:
    ///
    /// * **Before the park.** The tid is published from *outside* the critical section —
    ///   [`LoopbackRm::verb`] takes the client lock afterwards — so a received tid says
    ///   only that the thread is running. A sibling that wins the lock first completes
    ///   **legitimately, before any verb was parked**.
    /// * **After the release.** `verb` drops the guard on `return`, and only *then* does
    ///   the thread send its result. A sibling that takes the freed lock in that gap has
    ///   also completed legitimately, **after the park ended** — but the sampling loop
    ///   scores it as progress during the park, because the parker's answer has not landed
    ///   yet.
    ///
    /// Both windows produce the identical message, which is why the report was ambiguous.
    /// So the sample below is taken **once, strictly inside the interval where it is
    /// sound**: the parked verb is proven to hold the lock, the sibling is proven to have
    /// reached that lock and found it held, and **no break signal has been delivered yet** —
    /// and a break is the only thing that can end a park, so the lock cannot have moved.
    #[test]
    fn a_parked_verb_holds_the_client_lock_against_its_own_siblings() {
        use std::sync::TryLockError;
        use std::sync::mpsc::TryRecvError;

        /// Backstop on the rendezvous below, never the mechanism: each is a condition that
        /// holds within microseconds when the fixture works, so reaching this means the
        /// thread never arrived at all — and it then FAILS by name instead of hanging.
        const NEVER_ARRIVED: std::time::Duration = std::time::Duration::from_secs(30);

        kayfabe_linux_raw::install_break_handler().expect("handler");
        let shared = LoopbackShared::new(ParkVerb::Sysmem, None).expect("pipe");
        let id = IsolateId::new(1, GpuId(0));
        let mut parker = LoopbackRm::new(id, Arc::clone(&shared), exports()).expect("dup");
        let mut sibling = LoopbackRm::new(id, Arc::clone(&shared), exports()).expect("dup");

        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        let (parked_tx, parked_rx) = std::sync::mpsc::channel();
        let a = std::thread::spawn(move || {
            tid_tx
                .send(kayfabe_linux_raw::current_thread_id())
                .expect("tid");
            parked_tx.send(parker.alloc_sysmem(0x1000)).expect("send");
        });
        let tid = tid_rx.recv().expect("tid");

        // ---- ★ PRECONDITION 1: the parked verb really holds the client lock. `try_lock`
        // is the direct observation of exactly that, and it is the observation the tid is
        // NOT: nothing else has taken this lock, so `WouldBlock` means the parker has it.
        let deadline = std::time::Instant::now() + NEVER_ARRIVED;
        loop {
            match shared.client.try_lock() {
                Err(TryLockError::WouldBlock) => break,
                // Still free — the parker has not reached the lock. Release it at once so
                // it can, rather than becoming the thing that blocks it.
                Ok(guard) => drop(guard),
                Err(TryLockError::Poisoned(_)) => panic!("the client lock is poisoned"),
            }
            assert!(
                std::time::Instant::now() < deadline,
                "★ the parked verb never took the client lock within {NEVER_ARRIVED:?}, so \
                 nothing below would have been about a park at all"
            );
            std::thread::yield_now();
        }

        // ---- ★ PRECONDITION 2: the sibling really reaches that lock, and finds it HELD.
        // Reported from the sibling's OWN side, because otherwise the emptiness asserted
        // below is satisfied just as well by a sibling that had not started yet — the
        // vacuous pass this test would otherwise be one scheduling decision away from.
        let shared_b = Arc::clone(&shared);
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let (sib_tx, sib_rx) = std::sync::mpsc::channel();
        let b = std::thread::spawn(move || {
            contended_tx
                .send(matches!(
                    shared_b.client.try_lock(),
                    Err(TryLockError::WouldBlock)
                ))
                .expect("contended");
            // The sibling issues a NON-parking verb. It must still block, because the lock
            // is per client and not per verb.
            sib_tx.send(sibling.alloc_vaspace()).expect("send");
        });
        assert_eq!(
            contended_rx.recv_timeout(NEVER_ARRIVED),
            Ok(true),
            "★ NON-VACUITY: the sibling must have reached the client lock and found it HELD"
        );

        // ---- ★ THE ASSERTION, in the interval where it is sound. No break has been
        // delivered, and a break is the only thing that ends a park, so the parked verb
        // still holds the lock — and the sibling is blocked on it rather than absent.
        assert_eq!(
            sib_rx.try_recv(),
            Err(TryRecvError::Empty),
            "a sibling on the SAME client made progress while a verb was parked"
        );

        // ---- Deliver the break until the parked verb reports it.
        let parked_result = loop {
            assert!(kayfabe_linux_raw::interrupt_thread(tid).expect("tgkill"));
            if let Ok(r) = parked_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                break r;
            }
        };
        assert_eq!(parked_result, Err(RmError::Interrupted));

        // ---- ★ And once released, the sibling completes — with the isolate's FIRST
        // handle. That exact value is the other half of the statement and is not a clock:
        // the interrupted verb held the lock and parked, but it returned before the mint,
        // so it consumed no handle and the sibling gets `next` untouched.
        assert_eq!(
            sib_rx.recv_timeout(NEVER_ARRIVED),
            Ok(Ok(HostHandle::new(id, 0x00CA_FE01))),
            "★ the sibling's verb completed once the lock was released, and it minted the \
             isolate's FIRST handle — so the verb that parked and was interrupted took none"
        );
        a.join().expect("join a");
        b.join().expect("join b");
    }
}
