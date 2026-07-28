//! ★★★ Retirement, in its own test binary, and the **asymmetry** that is finding 2.
//!
//! Two properties, and the whole point is that they are opposites:
//!
//! 1. **Our own reservation** is released by the *machine*, never by an accessor — #57's
//!    ownership fix, inherited verbatim. The `Arc` an accessor holds so its copy can run
//!    outside the view lock is also a **release** handle, and `gpa_read` is entered with
//!    one of the core's ranked locks held, so an accessor that ran the release would be
//!    unmapping under a rank. That is R1.
//! 2. **A hypervisor-owned region** is released by the *topology callback*, on its own
//!    thread, with no deferral at all — because the copy out of one runs **inside** the
//!    view lock, so no accessor can be holding it when the callback lands. Deferring that
//!    release, as `l2_qemu_adapter.md` §0 item 1 prescribes, would move a finalizer off
//!    the one thread that holds the lock a finalizer needs.
//!
//! ## Why this is not in `memory_plane.rs`
//!
//! The same measured reason the KVM backend's equivalent gives: making the interleaving
//! deterministic needs a reservation large enough that the copy is still running long
//! after the removal lands, and two tests in one binary run on two threads of one process.
//! Cargo runs integration-test targets one at a time, so a separate binary is the
//! isolation.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use kayfabe_util::lockwitness;
use kayfabe_vmm::{BarId, Vmm};
use kayfabe_vmm_qemu::host::{BarPlacement, SectionFacts};
use kayfabe_vmm_qemu::mock_host::{HostCall, MockPolicy};
use kayfabe_vmm_qemu::{MachineConfig, WindowSpec};

const BIG_BAR: u64 = 0x9000_0000;
const BIG_LEN: u64 = 64 * 1024 * 1024;

/// ★ A backstop on the rendezvous spin — emphatically **not** the rendezvous.
///
/// The edge this test synchronises on is clock-free (`accesses_served != 0`). What this
/// bounds is the case where that edge is never reached at all — the reader panicking
/// inside the resolve, or a refactor that stops bumping the counter. With no bound the
/// main thread spins forever at 100 % of a core and libtest prints **nothing**, because
/// the reader's panic sits in a capture buffer that is flushed only when the test ends. A
/// hang tells you nothing; a red test tells you something.
const RENDEZVOUS_DEADLINE: Duration = Duration::from_secs(60);

/// ★★★ A reservation released while an accessor is still copying out of it is released by
/// the **machine**, at a door already proved lock-free — never by the accessor.
#[test]
fn a_reservation_torn_down_under_a_live_reader_is_never_unmapped_by_the_reader() {
    let cfg = MachineConfig {
        shareable_ram: true,
        bars: vec![BarPlacement {
            bar: BarId::Bar1,
            base: BIG_BAR,
            len: BIG_LEN,
        }],
        windows: vec![WindowSpec {
            bar: BarId::Bar1,
            gpa: BIG_BAR,
            len: BIG_LEN,
        }],
        overlays: Vec::new(),
        traps: Vec::new(),
    };
    let (m, host) = common::machine_with(MockPolicy::default(), cfg);

    let mut buf = vec![0u8; usize::try_from(BIG_LEN).expect("fits")];
    std::thread::scope(|s| {
        let reader = s.spawn(|| {
            let mut v = m.vmm();
            // Entered exactly as the core's route phase enters it: WITH rank 0 held. That
            // is legal — and it is what makes an unmap on this thread a violation rather
            // than a slow path.
            lockwitness::note_acquired(0);
            let r = v.gpa_read(BIG_BAR, &mut buf);
            lockwitness::note_released(0);
            r
        });
        let deadline = Instant::now() + RENDEZVOUS_DEADLINE;
        while m.audit().accesses_served == 0 {
            assert!(
                Instant::now() < deadline,
                "★ the reader never served an access within {RENDEZVOUS_DEADLINE:?} — it \
                 died before the resolve handed it the reservation, or the counter is no \
                 longer bumped. FAIL here rather than spin forever: the reader thread's \
                 own panic message is printed only when this test ENDS, so a hang here is \
                 silent by construction"
            );
            core::hint::spin_loop();
        }
        // §8.3's teardown, landing on a reservation somebody is mid-copy out of.
        m.unrealize();
        assert_eq!(
            reader.join().expect(
                "the reader thread panicked — with the retirement removed this is R1 \
                 firing on an unmap under rank 0, which is the whole point of the test"
            ),
            Ok(()),
            "the reader had already resolved, so its copy must complete against memory \
             that is still mapped"
        );
    });

    let a = m.audit();
    assert_eq!(
        a.window_releases_deferred, 1,
        "★ NON-VACUITY: the teardown really did land on a reservation an accessor was \
         still holding. Without this the assertion below is equally true of a run in \
         which the reader finished first and nothing was ever deferred"
    );
    assert_eq!(
        a.window_mappings_released, 0,
        "★ and the release did NOT happen during unrealize: the reader still held the \
         mapping, so no thread present was allowed to unmap it"
    );
    assert_eq!(
        a.syscall_ranked_depth,
        (0, 0),
        "and nothing syscall-shaped ran under a rank while all of that happened"
    );

    // The next door that is lock-free by contract collects it — and only then.
    m.register_backing(4096).expect("a lock-free door");
    assert_eq!(
        m.audit().window_mappings_released,
        1,
        "★ the parked mapping is released by the machine at the next lock-free door — \
         parked, not leaked. A retirement nobody collects is an unmap that never happens"
    );
    assert!(
        host.live_regions().is_empty(),
        "and the hypervisor got its region back regardless of the deferral, because the \
         two releases are independent: ours is an unmap, its is a reference"
    );
}

/// ★★★ The **other half of the asymmetry**: a hypervisor-owned region is released inline,
/// on the callback's own thread, and is never deferred.
///
/// Same shape as above — a reader holding a rank, a removal landing on it — but the
/// backing is one the topology listener reported. Because the copy out of it runs *inside*
/// the view lock, `region_del` cannot land mid-copy at all: it either runs before the
/// accessor takes the lock or after the accessor has released it. So the reference falls
/// on the callback's thread, with **zero** deferrals, which is exactly what §0 item 1's
/// prescription would have prevented.
#[test]
fn a_hypervisor_owned_region_is_released_inline_and_is_never_deferred() {
    let (m, host) = common::machine();
    let p = common::page();
    let section = host.mint_foreign(common::FOREIGN_RAM, 1024 * p, SectionFacts::plain_ram());
    m.region_add(section).expect("guest RAM");

    let m = Arc::new(m);
    let reader_m = Arc::clone(&m);
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader_done = Arc::clone(&done);
    let reader = std::thread::spawn(move || {
        let mut v = reader_m.vmm();
        let mut buf = vec![0u8; usize::try_from(1024 * common::page()).expect("fits")];
        let mut last = Ok(());
        for _ in 0..64 {
            lockwitness::note_acquired(0);
            last = v.gpa_read(common::FOREIGN_RAM, &mut buf);
            lockwitness::note_released(0);
            if last.is_err() {
                break;
            }
        }
        reader_done.store(true, std::sync::atomic::Ordering::SeqCst);
        last
    });

    let deadline = Instant::now() + RENDEZVOUS_DEADLINE;
    while m.audit().accesses_served == 0 {
        assert!(
            Instant::now() < deadline,
            "★ the reader never served an access within {RENDEZVOUS_DEADLINE:?}"
        );
        core::hint::spin_loop();
    }
    m.region_del(common::FOREIGN_RAM, 1024 * p);
    let last = reader.join().expect("the reader thread must not panic");
    assert!(
        done.load(std::sync::atomic::Ordering::SeqCst),
        "the reader ran to completion"
    );
    assert!(
        last == Ok(())
            || last
                == Err(kayfabe_vmm::VmmError::BadGpa {
                    gpa: common::FOREIGN_RAM
                }),
        "★ a read either completes against a live region or refuses because the range was \
         undeclared FIRST. What it must never be is a copy against a region whose last \
         reference has already fallen — and there is no third outcome ({last:?})"
    );

    assert_eq!(
        m.audit().window_releases_deferred,
        0,
        "★★ ZERO deferrals. A hypervisor-owned region is never retired, because the copy \
         runs inside the view lock and the callback therefore cannot land mid-copy. \
         Deferring it — which §0 item 1 prescribes for every destructor-shaped foreign \
         call — would move the release onto a thread that does not hold the global lock a \
         finalizer needs, which is worse than the problem it solves"
    );
    assert_eq!(
        host.live_regions()
            .iter()
            .find(|(h, _)| *h == section.mr)
            .map(|(_, r)| *r),
        Some(0),
        "and the reference fell exactly once"
    );
    assert_eq!(
        host.log()
            .iter()
            .filter(|c| **c == HostCall::UnrefRegion(section.mr))
            .count(),
        1,
        "★ exactly once — a release performed twice is a use-after-free in the hypervisor \
         and a release performed zero times is a region it can never finalize"
    );
    assert!(
        m.audit().host_copy_leaf_depth_min >= 1,
        "★ NON-VACUITY of the mechanism this whole test rests on: the copies really did \
         run inside the view lock. If they stop, the reasoning above stops with them"
    );
}
