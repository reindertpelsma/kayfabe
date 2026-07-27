//! ★★★ **The window-retirement property, in its own test binary** — `l1_os_shell.md`
//! §14.8 F7(b)'s 2026-07-27 correction.
//!
//! One test, one property: *a window removed while an accessor is still reading it is
//! released by the **machine**, never by the accessor.* The accessor holds an
//! `Arc<GuestWindow>` so its copy can run outside the view lock (§14.8 F2) — and that `Arc`
//! **owns** the mapping, so if it is the last one the accessor runs `munmap`, under the
//! ranked lock `gpa_read` is legally entered with. That is R1 (`l1_concurrency.md` §3.3),
//! and it is what `l1_mean.rs`'s composed run hit about one time in 60–100.
//!
//! ## ★ Why this is not in `memory_plane.rs`
//!
//! Not taste — a measured collision. Making the interleaving deterministic needs a window
//! large enough that the copy is still running long after the removal lands, and
//! `memory_plane.rs::a_repeated_partial_failure_returns_the_host_address_space` asserts on
//! **`VmSize`, a process-wide number**, with an 8 MiB bar. Two tests in one binary run on
//! two threads of one process, so a 64 MiB window here makes that test fail — an instrument
//! reading another test's address space, which is a false red rather than a finding. Cargo
//! runs integration-test **targets** one at a time, so a separate binary is the isolation.
//! Anything added here that maps memory at this scale carries the same constraint.

use kayfabe_linux_raw::HostPageSize;
use kayfabe_util::lockwitness;
use kayfabe_vmm::{BarId, Vmm};
use kayfabe_vmm_kvm::{BarPlacement, KvmMachine, MachineConfig};

const GPA_RAM: u64 = 0x1000_0000;
const GPA_BAR0: u64 = 0x7000_0000;
const BAR0_LEN: u64 = 0x1_0000;

fn page() -> HostPageSize {
    HostPageSize::query()
}

/// The same machine shape `memory_plane.rs` uses, so the two files describe one device.
fn machine() -> KvmMachine {
    KvmMachine::realize(MachineConfig {
        shareable_ram: true,
        bars: vec![BarPlacement {
            bar: BarId::Bar0,
            base: GPA_BAR0,
            len: BAR0_LEN,
        }],
    })
    .expect(
        "/dev/kvm must be present and permitted for the KVM-direct harness (§10, decision \
         #48) — a deployment fact no code gate can observe, so it refuses loudly here",
    )
}

/// ★★★ **A window removed under a live reader is released by the MACHINE, never by the
/// reader** — the R1 violation M2-c shipped, and the half `l1_os_shell.md` §14.8 F7(b)
/// walked past when it wrote *"the `Arc` keeps the mapping alive either way"*.
///
/// The `Arc` `resolve` hands an accessor is a **release** handle as well as a read handle.
/// `gpa_read` is in-lock *legal* — entered with one of the core's ranked locks held — so if
/// the accessor's clone is the last one, `GuestWindow::drop` runs `munmap` under that rank,
/// which is R1 (`l1_concurrency.md` §3.3). In the composed mean run it fired about one run
/// in six, in a thread doing `parse_pushbuffer` while the main thread removed its window:
///
/// ```text
/// R1 no-blocking-under-lock violation: munmap (dropping a guest-physical window)
///   while holding rank(s) [0]
/// ```
///
/// Here it is deterministic rather than one-in-six. **The rendezvous uses no clock:**
/// `accesses_served` is bumped after `resolve` has handed the reader its `Arc` and *before*
/// the memcpy, so the spin below ends exactly when the reader is inside the copy holding
/// the mapping alive. The window is large so that the copy is still running long after the
/// removal lands — and `window_releases_deferred` is asserted, so a run in which the reader
/// finished early **fails** instead of passing vacuously about an interleaving it never
/// reached.
#[test]
fn a_window_removed_under_a_live_reader_is_never_munmapped_by_the_reader() {
    kayfabe_linux_raw::require_kvm!(
        "a_window_removed_under_a_live_reader_is_never_munmapped_by_the_reader"
    );
    let m = machine();
    let p = page();
    let len: u64 = 64 * 1024 * 1024;
    let region = m
        .install_ram_window(GPA_RAM, len)
        .expect("a large window installs");

    let mut buf = vec![0u8; usize::try_from(len).expect("fits")];
    std::thread::scope(|s| {
        let reader = s.spawn(|| {
            let mut v = m.vmm();
            // Entered exactly as `SharedDevice::parse_pushbuffer`'s route phase enters it:
            // WITH rank 0 held. That is legal (§6.1) — and it is what makes a `munmap` on
            // this thread a violation rather than a slow path.
            lockwitness::note_acquired(0);
            let r = v.gpa_read(GPA_RAM, &mut buf);
            lockwitness::note_released(0);
            r
        });
        while m.audit().accesses_served == 0 {
            core::hint::spin_loop();
        }
        m.remove_window(region)
            .expect("★ a REAL memslot DELETE while a reader is copying out of the window");
        assert_eq!(
            reader.join().expect(
                "the reader thread panicked — with the retirement removed this is R1 \
                 firing on `munmap` under rank 0, which is the whole point of the test"
            ),
            Ok(()),
            "the reader had already resolved, so its copy must complete against memory \
             that is still mapped (§14.8 F2's soundness argument)"
        );
    });

    let a = m.audit();
    assert_eq!(
        a.window_releases_deferred, 1,
        "★ NON-VACUITY: the removal really did land on a window an accessor was still \
         holding. Without this the assertion below is equally true of a run in which the \
         reader finished first and nothing was ever deferred"
    );
    assert_eq!(
        a.window_mappings_released, 0,
        "★ and the release did NOT happen at `remove_window`: the reader still held the \
         mapping, so there was no thread present that was allowed to `munmap` it"
    );

    // The next door that is lock-free by contract collects it — and only then.
    m.register_backing(p.bytes()).expect("a lock-free door");
    assert_eq!(
        m.audit().window_mappings_released,
        1,
        "★ the parked mapping is released by the machine at the next lock-free door — \
         parked, not leaked. A retirement nobody collects is a `munmap` that never happens"
    );
}
