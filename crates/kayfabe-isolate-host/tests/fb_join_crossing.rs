//! ★★★★★ **ONE MEMORY across the process boundary** — the join, end to end, against a real
//! spawned isolate, with **no GPU** (`fb_cpu_view.md` §4).
//!
//! `export_backing.rs` proved that what crosses is memory the VMM cannot `ioctl`, and stated
//! its own bound in as many words:
//!
//! > *"⊘ What it does **not** prove: that the **isolate** can see the write. Nothing in the
//! > port reads the isolate's own view of a backing, so that half is unmeasured here."*
//!
//! This file pays that bound. [`kayfabe_isolate::RmBackend::fb_join_peek`] is the port that
//! reads the isolate's own view, so for the first time the two halves of a backing can be
//! compared **from opposite processes** rather than from two duplicates in one.
//!
//! ## ★★★ THE TRAP THIS FILE EXISTS FOR: an isolate is a POOL
//!
//! Every test here runs a pool of **four** workers and deliberately joins on one and reads on
//! others. A join table living on the backend rather than on the isolate would be correct on
//! every one-worker test — Clippy, the whole non-GPU suite and a brand-new falsifier were all
//! green while exactly that was broken one plane over, because they ran one worker. ⊘ A
//! `pool_size(1)` here would make this file vacuous, and that is why the size is named in
//! [`POOL`] with this paragraph attached to it.
//!
//! ## ⊘ What runs here is HALF the chain, and the half is named
//!
//! `RmMode::Loopback` mints the real `memfd`, performs the real `mmap` in the child, and keeps
//! the real join table — so the two-views property is genuinely measured. It has **no RM and
//! no GPU MMU**, so `memory` is a fixture handle and `host_va` is `at` by fiat. ⇒ Green here
//! says the VMM's plumbing works; it says **nothing** about whether RM would place the
//! mapping, which is `fb_cpu_view.md` §3's hardware measurement.

use kayfabe_arch::ids::{GpuId, GpuVa};
use kayfabe_isolate::{
    FbLeafJoined, Isolate as _, IsolateId, RmError, VerbPlan, VerbReply, Worker,
};
use kayfabe_isolate_host::{HostIsolate, HostIsolateFactory, ParkVerb, RmMode};
use kayfabe_linux_raw::{Backing, HostOffset, HostPageSize, HostProt, MappedRegion};

/// ★★★ **Four workers, and the number is the point.** See the module docs: an isolate is a
/// pool, and a join and the read of it need not land on the same slot. A one-worker pool
/// would make every assertion in this file true of a per-worker table too.
const POOL: usize = 4;

/// The leaf under test: length, guest VA, framebuffer address.
///
/// ⊘ A whole 64 KiB granule — the unit RM can place exactly — even though the loopback
/// backend does not place anything. The fixture must not be able to pass with a geometry the
/// production chain would refuse.
const LEN: u64 = 0x1_0000;
const AT: GpuVa = GpuVa(0x2_0020_0000);
const PHYS: u64 = 0x40_0000;

/// ★ The descriptor table is process-wide and libtest runs tests as threads of one process;
/// spawning an isolate mutates a neighbour's view of it. Inherited from `export_backing.rs`
/// verbatim rather than rediscovered.
static FD_TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    FD_TABLE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Spawn a real isolate with a real pool.
fn isolate(id: IsolateId) -> HostIsolate {
    let factory = HostIsolateFactory::new(RmMode::Loopback)
        .with_park(ParkVerb::Nothing)
        .with_pool_size(POOL);
    let iso = factory.spawn_host(id);
    assert!(
        iso.spawn_error().is_none(),
        "the isolate did not start: {:?}",
        iso.spawn_error()
    );
    iso
}

/// Check **every** worker out, hand them to `f` as a slice, and check them all back in.
///
/// ★ All of them at once, deliberately: it is the only way a test can be sure the worker it
/// reads on is not the worker it joined on. Checking out twice in sequence hands back the
/// same idle slot, which is exactly the shape that made the pool bug invisible.
fn with_all_workers<T>(iso: &mut HostIsolate, f: impl FnOnce(&mut [Worker]) -> T) -> T {
    let mut ws: Vec<Worker> = (0..POOL)
        .map(|i| iso.checkout().unwrap_or_else(|| panic!("worker {i}")))
        .collect();
    assert_eq!(ws.len(), POOL, "the whole pool must be checked out");
    let out = f(&mut ws);
    for w in ws {
        iso.checkin(w);
    }
    out
}

/// ★ Run the join chain on `w` exactly as the core does — through
/// [`kayfabe_isolate::Worker::execute`], with the plan the forwarding layer emits.
///
/// ⊘ Not a direct backend call: `execute` is where the foreign-handle gate, the R1 assertion
/// and the address-identity check live, and a test that went round it would be exercising a
/// path no boot takes. `host_vas: None` lets the chain allocate its own VAS, which is the
/// arm a first join on a fresh address space takes.
fn join_on(w: &mut Worker) -> FbLeafJoined {
    match w.execute(&VerbPlan::JoinFbLeaf {
        host_vas: None,
        len: LEN,
        at: AT,
        phys: PHYS,
    }) {
        Ok(VerbReply::FbLeafJoined { joined, .. }) => joined,
        other => panic!("the join chain must answer FbLeafJoined, got {other:?}"),
    }
}

/// A per-word image. ⊘ Never a repeated constant: a whole-buffer compare against one repeated
/// word passes on any single correct word, and a truncated or misaddressed read would match.
fn image(base: u32, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    for (i, w) in v.chunks_exact_mut(4).enumerate() {
        w.copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
    }
    v
}

/// The VMM's own view of the backing, mapped the way the shim maps it.
fn vmm_view(iso: &HostIsolate, token: u64, shared: bool) -> MappedRegion {
    let fd = iso.exports().dup(token).expect("the VMM adopts the backing");
    MappedRegion::map(
        // ★★ THE ONE PROPERTY THE CONTROL CHANGES. Everything either side is identical.
        if shared {
            Backing::SharedFile {
                fd: std::os::fd::AsFd::as_fd(&fd),
                offset: 0,
            }
        } else {
            Backing::PrivateAnonymous
        },
        LEN,
        HostProt::ReadWrite,
        kayfabe_linux_raw::CachePolicy::WriteBack,
        HostPageSize::query(),
    )
    .expect("the VMM maps it")
}

// =====================================================================================
// 1 — ★★★★★ THE JOIN, BOTH DIRECTIONS, ACROSS TWO PROCESSES AND FOUR WORKERS
// =====================================================================================

/// ★★★★★ **The measurement this rung is for.** A pattern written through the VMM's view is
/// read through the isolate's, and a pattern written through the isolate's is read through the
/// VMM's — over ONE fabricated backing, from two processes, on three different pool slots.
#[test]
fn one_backing_two_processes_and_the_bytes_agree_in_both_directions() {
    let _fd_table = serialized();
    let mut iso = isolate(IsolateId::new(71, GpuId(0)));

    let (joined, g2h, h2g) = with_all_workers(&mut iso, |ws| {
        // ---- The join, on worker 0.
        let joined = join_on(&mut ws[0]);
        (joined, image(0xa19a_5a5b, 4096), image(0x043f_fffe, 4096))
    });
    assert_eq!(
        joined.backing.len, LEN,
        "the backing covers the whole leaf; a short one leaves part of it in two memories"
    );
    assert_eq!(joined.host_va, AT.0, "address identity");

    // ---- DIRECTION 1: the VMM writes, the ISOLATE reads. Run first because it is the
    // ESTABLISHMENT direction — bytes the guest already wrote must be visible to what RM is
    // about to describe.
    let view = vmm_view(&iso, joined.backing.token, true);
    view.write_from(HostOffset::new(0), &g2h)
        .expect("the VMM writes through its own mapping");

    let mut got = vec![0u8; 4096];
    let covered = with_all_workers(&mut iso, |ws| {
        // ★★★ WORKER 3, NOT WORKER 0. The join was made on another slot, in another
        // `checkout`. A per-worker table answers `Ok(false)` here, and this line is the whole
        // reason the pool is four wide.
        ws[3]
            .fb_join_peek(PHYS, &mut got, Some(0x043f_fffe))
            .expect("the peek is served")
    });
    assert!(
        covered,
        "★★★ the isolate must hold this range on EVERY worker — a `false` here is the \
         per-worker-table bug, and it is invisible to a one-worker test"
    );
    assert_eq!(
        got, g2h,
        "★ DIRECTION 1: what the VMM wrote is what the isolate's own mapping holds"
    );

    // ---- DIRECTION 2: the poke above was the isolate's write. Read it back here.
    let mut back = vec![0u8; 4096];
    view.read_into(HostOffset::new(0), &mut back)
        .expect("the VMM reads");
    assert_eq!(
        back, h2g,
        "★ DIRECTION 2: what the isolate wrote is what the VMM's mapping reads"
    );
}

// =====================================================================================
// 2 — ★★★★★ THE NEGATIVE CONTROL
// =====================================================================================

/// ★★★★★ **Watched to fail, and its fail arm returns the OTHER direction's pattern rather
/// than zeros.**
///
/// # Which line do I expect this to execute?
///
/// `Backing::PrivateAnonymous`'s arm of the mapping's `mmap` argument computation
/// (`crates/kayfabe-linux-raw/src/mapping_unsafe.rs:344-347`), yielding
/// `MAP_PRIVATE|MAP_ANONYMOUS` instead of `MAP_SHARED`. ⊘ **Not "a second memfd"**, which
/// would be a tautology — two different files obviously hold different bytes. The control
/// changes **only** the one property that makes two mappings one memory; the isolate chain,
/// the join table and both probes either side of it are the same code.
///
/// ★★ And read the second assertion, which is the strongest signal in this file: the VMM-side
/// read does **not** return zeros. It returns *direction 1's* pattern, still sitting in the
/// private pages this run wrote it into, because direction 2's write went to the memfd and
/// never reached them. A control that merely returned zeros would be consistent with a
/// mapping that was never written at all; this one demonstrates both views are live, hold
/// different bytes, and are read by the same loop.
#[test]
fn the_control_fires_and_the_vmm_reads_back_direction_ones_pattern_not_zeros() {
    let _fd_table = serialized();
    let mut iso = isolate(IsolateId::new(72, GpuId(0)));

    let joined = with_all_workers(&mut iso, |ws| {
        join_on(&mut ws[1])
    });

    let g2h = image(0xa19a_5a5b, 4096);
    let h2g = image(0x043f_fffe, 4096);

    // ⊘ PRIVATE. The one property.
    let view = vmm_view(&iso, joined.backing.token, false);
    view.write_from(HostOffset::new(0), &g2h).expect("writes");

    let mut got = vec![0u8; 4096];
    let covered = with_all_workers(&mut iso, |ws| {
        ws[2]
            .fb_join_peek(PHYS, &mut got, Some(0x043f_fffe))
            .expect("served")
    });
    assert!(
        covered,
        "⊘ the range is still joined — the control changes the VMM's MAPPING, not the join"
    );
    assert_ne!(
        got, g2h,
        "★ DIRECTION 1 must DISAGREE: the VMM wrote into private pages nobody else has"
    );
    assert_eq!(
        got,
        vec![0u8; 4096],
        "and what the isolate sees is its own untouched zero fill"
    );

    let mut back = vec![0u8; 4096];
    view.read_into(HostOffset::new(0), &mut back)
        .expect("reads");
    assert_ne!(back, h2g, "★ DIRECTION 2 must DISAGREE too");
    assert_eq!(
        back, g2h,
        "★★ AND IT RETURNS DIRECTION 1'S OWN PATTERN, NOT ZEROS — both views are live and \
         hold different bytes, which zeros alone could not have shown"
    );
}

// =====================================================================================
// 3 — ★★★ THE MISS, AND WHY IT IS NOT ZEROS
// =====================================================================================

/// ★★★ An address no join covers answers `Ok(false)` — *"nothing covers that range"* — and
/// never a page of zeros.
///
/// ⊘ Those are opposite findings: a joined range holding zeros is a leaf nobody has written,
/// and an unjoined range is a leaf the engine will read out of memory the guest cannot see. A
/// port that spelled the second as the first would report the two-memories defect as an empty
/// buffer.
#[test]
fn an_unjoined_framebuffer_address_is_a_miss_and_not_a_page_of_zeros() {
    let _fd_table = serialized();
    let mut iso = isolate(IsolateId::new(73, GpuId(0)));

    // Before any join at all — on every worker.
    let mut buf = [0u8; 64];
    with_all_workers(&mut iso, |ws| {
        for (i, w) in ws.iter_mut().enumerate() {
            assert_eq!(
                w.fb_join_peek(PHYS, &mut buf, None),
                Ok(false),
                "worker {i} must report a MISS before anything is joined"
            );
        }
        join_on(&mut ws[0]);
    });

    // After it: the joined range hits on every worker, and its neighbour still misses.
    with_all_workers(&mut iso, |ws| {
        for (i, w) in ws.iter_mut().enumerate() {
            assert_eq!(
                w.fb_join_peek(PHYS, &mut buf, None),
                Ok(true),
                "worker {i} must see the join made on worker 0"
            );
            assert_eq!(
                w.fb_join_peek(PHYS + LEN, &mut buf, None),
                Ok(false),
                "worker {i}: one leaf past the join is still a miss — the table is not a \
                 wildcard"
            );
        }
    });
}

// =====================================================================================
// 4 — ★★ THE BACKING IS THE ISOLATE'S, AND IT IS MEMORY
// =====================================================================================

/// ★★ The descriptor a join hands up is subject to the **same** kernel check every export is:
/// [`kayfabe_isolate_host::ExportRegistry::adopt`] refuses anything that is not a regular
/// file, before it is reachable.
///
/// ⊘ Not redundant with `export_backing.rs`: that file quantifies over `Request::ExportBacking`
/// and this is a **second** request whose reply may carry a descriptor. A protocol-policy set
/// that grew from one member to two is exactly the shape a gate quantified over a list of one
/// misses.
#[test]
fn a_joins_descriptor_is_checked_by_the_kernel_like_every_other_export() {
    let _fd_table = serialized();
    let mut iso = isolate(IsolateId::new(74, GpuId(0)));
    let joined = with_all_workers(&mut iso, |ws| join_on(&mut ws[0]));
    assert_eq!(
        iso.exports().kind(joined.backing.token),
        Some(kayfabe_linux_raw::DescriptorKind::RegularFile),
        "★ what crossed must be MEMORY — a character device here is the design decision (b) \
         exists to prevent, arriving through the newer of the two doors"
    );
    let fd = iso.exports().dup(joined.backing.token).expect("dup");
    let raw = std::os::fd::AsRawFd::as_raw_fd(&std::os::fd::AsFd::as_fd(&fd));
    let target = std::fs::read_link(format!("/proc/self/fd/{raw}"))
        .expect("readlink")
        .to_string_lossy()
        .into_owned();
    assert!(
        target.starts_with("/memfd:"),
        "★★ and /proc must say so independently of what `adopt` recorded; it is {target}"
    );
}

/// ⊘ A backend with **no shared join table** refuses by name rather than minting a private
/// one. There is no such backend in a spawned isolate — `child.rs` builds exactly one table
/// and clones it into every worker — so this asserts the refusal exists at all, against a
/// backend built the way a diagnostic or a future caller might build one.
#[test]
fn a_backend_with_no_shared_join_table_refuses_by_name() {
    let _fd_table = serialized();
    // ⊘ `LoopbackRm` without `with_fb_joins`. The point is that the absence is a REFUSAL and
    // not a silently-minted per-worker table, which would be correct here and wrong on a boot.
    let shared = kayfabe_isolate_host::loopback::LoopbackShared::new(ParkVerb::Nothing, None)
        .expect("park pipe");
    let exports = std::sync::Arc::new(kayfabe_isolate_host::ChildExports::new());
    let mut rm = kayfabe_isolate_host::loopback::LoopbackRm::new(
        IsolateId::new(75, GpuId(0)),
        shared,
        exports,
    )
    .expect("builds");
    use kayfabe_isolate::RmBackend as _;
    let e = rm
        .join_fb_leaf(
            kayfabe_isolate::HostHandle::new(IsolateId::new(75, GpuId(0)), 0),
            LEN,
            AT,
            PHYS,
        )
        .expect_err("must refuse");
    assert_eq!(
        e,
        RmError::Other(kayfabe_isolate_host::rm::FB_JOIN_NO_TABLE),
        "★ by name, and specifically NOT by minting a table this worker alone can see"
    );
    let mut buf = [0u8; 4];
    assert_eq!(
        rm.fb_join_peek(PHYS, &mut buf, None),
        Err(RmError::Other(kayfabe_isolate_host::rm::FB_JOIN_NO_TABLE)),
        "⊘ and the instrument refuses too — never `Ok(false)`, which would say the isolate \
         looked and found nothing"
    );
}
