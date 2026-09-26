//! ★★ The real memory plane, end to end — `l1_os_shell.md` §6.1/§6.2/§6.7, stage M2-c.
//!
//! Every test here drives the **real** thing: a real `/dev/kvm` descriptor, real
//! `KVM_SET_USER_MEMORY_REGION` memslots, real `mmap`ed windows and real `MAP_FIXED`
//! placements. The arms they assert — `EEXIST` for an overlapping guest-physical range,
//! `ENOMEM` for a window the address space cannot hold, a real memslot ceiling — are arms
//! `MockVmm` does not have and cannot be given: its `map_guest` is a `BTreeMap::insert`.
//! That is gap **O1**, and closing it is what this stage is for.
//!
//! The composed mean run lives in `tests/tests/l1_mean.rs`, where the doctrine says it
//! belongs; these are the focused siblings that name one property each.

use core::time::Duration;

use kayfabe_linux_raw::HostPageSize;
use kayfabe_util::lockwitness;
use kayfabe_vmm::{
    BarId, CoreEvent, CoreEventKind, HostRegion, IrqSpec, Prot, TrapMode, Vmm, VmmError,
};
use kayfabe_vmm_kvm::{BarPlacement, KvmMachine, MachineConfig, leaf};

const GPA_RAM: u64 = 0x1000_0000;
const GPA_BAR0: u64 = 0x7000_0000;
/// ★ Four MiB, not 64 KiB. A read-native overlay's write-trap span must lie inside a
/// realized BAR (#87), and the span is rounded out to whole **host** pages — so a BAR
/// sized in 4 KiB units stops being able to hold a multi-page overlay on a 64 KiB-page
/// arm64 host. Sizing it in host-page-agnostic megabytes is what keeps these tests about
/// the overlay rather than about the page size they happened to run on.
const BAR0_LEN: u64 = 0x40_0000;

fn page() -> HostPageSize {
    HostPageSize::query()
}

/// A machine with one BAR declared, as a realized device would have.
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

// =================================================================================
// The plane's ordinary life
// =================================================================================

/// ★ A window is a **memslot plus a mapping**, and a placement inside it is neither.
/// This is §6.7's frequency rule as an observation rather than a rule: ten publications
/// into one window perform **one** memslot install.
#[test]
fn ten_publications_into_one_window_perform_exactly_one_memslot_install() {
    kayfabe_linux_raw::require_kvm!(
        "ten_publications_into_one_window_perform_exactly_one_memslot_install"
    );
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let region = m
        .install_ram_window(GPA_RAM, 16 * p.bytes())
        .expect("a 16-page window installs");

    let backing = m.register_backing(p.bytes()).expect("a host backing");
    let mut slots = Vec::new();
    for i in 0..10u64 {
        slots.push(
            v.map_guest(GPA_RAM + i * p.bytes(), p.bytes(), backing, Prot::ReadWrite)
                .expect("a publication into an installed window"),
        );
    }

    let a = m.audit();
    assert_eq!(
        (a.memslot_installs, a.placements_made),
        (1, 10),
        "★ THE MEMSLOT-FREQUENCY GATE (§9.3): installs must scale with WINDOWS and \
         placements with PUBLICATIONS. A per-object memslot would read (11, 10) here — \
         and that is the C artifact's measured regression (>1500 slots for one \
         cuCtxCreate), caught structurally and without a clock"
    );
    assert_eq!(
        (a.live_windows, a.live_memslots, a.live_placements),
        (1, 1, 10)
    );

    for s in slots {
        v.unmap_guest(s).expect("restore");
    }
    m.remove_window(region).expect("remove");
    let a = m.audit();
    assert_eq!(
        (
            a.live_windows,
            a.live_memslots,
            a.live_placements,
            a.window_bytes
        ),
        (0, 0, 0, 0),
        "the conservation ledger must balance after teardown"
    );
    assert_eq!(
        (a.peak_windows, a.peak_memslots, a.peak_placements),
        (1, 1, 10),
        "★ NON-VACUITY: the ledger's peaks prove it ever counted anything. Without this, \
         a ledger that was never incremented balances perfectly"
    );
}

/// ★ Guest memory that really is memory: written through the port, read back through the
/// port, at an offset deep inside the window.
#[test]
fn guest_memory_round_trips_through_a_real_mapping() {
    kayfabe_linux_raw::require_kvm!("guest_memory_round_trips_through_a_real_mapping");
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, 8 * p.bytes())
        .expect("window");

    let payload: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    // Deep inside the window, deliberately unaligned: a copy is a copy, and only the
    // MAPPING geometry is page-granular.
    let at = GPA_RAM + 3 * p.bytes() + 8;
    v.gpa_write(at, &payload).expect("write");
    let mut got = vec![0u8; payload.len()];
    v.gpa_read(at, &mut got).expect("read");
    assert_eq!(got, payload, "a real mapping must round-trip its bytes");

    // The whole window is readable before anything was published into it — a live memslot
    // names the range whether or not we filled it.
    let mut zeroes = [0xFFu8; 32];
    v.gpa_read(GPA_RAM, &mut zeroes).expect("read the head");
    assert_eq!(zeroes, [0u8; 32]);
}

// =================================================================================
// ★★ The region map versus the kernel — a consistency the mock cannot express
// =================================================================================

/// ★★★ On KVM a **device region IS the absence of a memslot**. So the guest-steerable
/// hazard (§10.1 item 6) is not modelled here, it is physical: an access to the BAR has
/// nowhere to land, and the port refuses it by the name that says so.
#[test]
fn a_device_region_is_the_absence_of_a_memslot_and_refuses_by_name() {
    kayfabe_linux_raw::require_kvm!(
        "a_device_region_is_the_absence_of_a_memslot_and_refuses_by_name"
    );
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, 2 * p.bytes())
        .expect("window");

    let mut buf = [0u8; 16];
    assert_eq!(
        v.gpa_read(GPA_BAR0, &mut buf),
        Err(VmmError::NonRamGpa { gpa: GPA_BAR0 }),
        "our own trapped BAR is a device window: serving it would DMA into our own MMIO \
         dispatch from inside a locked section"
    );
    assert_eq!(
        v.gpa_write(GPA_BAR0 + 0x800, &[1, 2, 3, 4]),
        Err(VmmError::NonRamGpa {
            gpa: GPA_BAR0 + 0x800
        }),
        "and the write direction is the sharper one — a stray write into a device \
         register window is a side effect on hardware"
    );
    assert_eq!(
        v.gpa_read(0xDEAD_0000, &mut buf),
        Err(VmmError::BadGpa { gpa: 0xDEAD_0000 }),
        "a hole is the NEAR NEIGHBOUR and must never start reporting as a device"
    );
    // The straddling shape: starts in real RAM, runs into nothing.
    assert_eq!(
        v.gpa_read(GPA_RAM + 2 * p.bytes() - 8, &mut buf),
        Err(VmmError::BadGpa {
            gpa: GPA_RAM + 2 * p.bytes()
        }),
        "a range that starts in RAM and leaves its region must report the BOUNDARY byte, \
         not its own start — a start-address-only check is not a check"
    );

    assert_eq!(
        m.assert_map_matches_the_kernel(),
        1,
        "★ NON-VACUITY: the map/kernel consistency check must have had a region to check"
    );
}

/// ★ The window's teardown makes the range stop resolving **before** the mapping goes
/// away, so a reader either resolves and completes or refuses. Never a stale byte.
#[test]
fn a_removed_window_refuses_as_unbacked_and_never_as_stale_bytes() {
    kayfabe_linux_raw::require_kvm!(
        "a_removed_window_refuses_as_unbacked_and_never_as_stale_bytes"
    );
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let region = m.install_ram_window(GPA_RAM, p.bytes()).expect("window");
    v.gpa_write(GPA_RAM, &[0xA5; 8]).expect("write");
    let mut got = [0u8; 8];
    v.gpa_read(GPA_RAM, &mut got).expect("read");
    assert_eq!(got, [0xA5; 8], "served before the teardown");

    m.remove_window(region).expect("remove");
    assert_eq!(
        v.gpa_read(GPA_RAM, &mut got),
        Err(VmmError::BadGpa { gpa: GPA_RAM }),
        "a torn-down window must refuse as UNBACKED — not as a device window, and never \
         as the bytes that used to be there"
    );
    // ★ And it was UNDECLARED, not merely disconnected. A bite-check found these two
    // states indistinguishable through `gpa_read` — both arrive as `BadGpa` — so the
    // region map is asserted directly. A stale RAM declaration is a guest-physical range
    // the map promises is memory and the kernel has no memslot for.
    assert_eq!(
        m.resolve_region(GPA_RAM, 8),
        Err(VmmError::BadGpa { gpa: GPA_RAM }),
        "the REGION was undeclared, not just its window forgotten"
    );
    assert_eq!(
        m.remove_window(region),
        Err(VmmError::BadSlot(kayfabe_vmm::SlotId(region.0))),
        "and removing it twice is a named refusal, not a second DELETE"
    );
}

// =================================================================================
// ★★ REAL SYSCALL FAILURES — the arms a mock does not have
// =================================================================================

/// ★★ `EEXIST` from the kernel: two windows may not claim one guest-physical range. The
/// adapter does not know this rule and does not need to — the flat view is the kernel's.
#[test]
fn an_overlapping_window_is_the_kernels_eexist_and_leaks_nothing() {
    kayfabe_linux_raw::require_kvm!(
        "an_overlapping_window_is_the_kernels_eexist_and_leaks_nothing"
    );
    let m = machine();
    let p = page();
    let first = m
        .install_ram_window(GPA_RAM, 4 * p.bytes())
        .expect("the first window");
    let before = m.audit();

    assert_eq!(
        m.install_ram_window(GPA_RAM + 2 * p.bytes(), 4 * p.bytes()),
        Err(VmmError::HostRefused {
            what: "a memslot install",
            errno: Some(libc::EEXIST),
        }),
        "the exact errno, not `is_err()`: EEXIST (an overlap we should have known about) \
         and EINVAL (an exhausted ceiling) are operationally different and must never \
         start reporting as each other"
    );

    let after = m.audit();
    assert_eq!(
        (
            after.live_windows,
            after.live_memslots,
            after.window_bytes,
            after.host_refusals
        ),
        (
            before.live_windows,
            before.live_memslots,
            before.window_bytes,
            before.host_refusals + 1
        ),
        "★ PARTIAL FAILURE LEAKS NOTHING: the mmap succeeded and the ioctl did not, so \
         the window must have been unmapped and nothing recorded — and the refusal must \
         have been COUNTED, or this assertion is about an operation that never ran"
    );
    // Proof the address space really was returned: the same GPA installs once the first
    // window is gone.
    m.remove_window(first).expect("remove");
    m.install_ram_window(GPA_RAM + 2 * p.bytes(), 4 * p.bytes())
        .expect("the range is free again");
    assert_eq!(m.assert_map_matches_the_kernel(), 1);
}

/// ★★ **The host address space is returned too, not merely the bookkeeping.**
///
/// This test exists because a bite-check found the previous one insufficient: leaking the
/// `mmap`ed window on the memslot-install failure path moved **no counter**, because the
/// ledger is only incremented on success. So "partial failure leaks nothing" was a claim
/// about the ledger and the guest-physical flat view, and said nothing about the process's
/// own address space — which is the resource that actually runs out.
///
/// `VmSize` from `/proc/self/status` is the instrument. Each refused install `mmap`s a
/// 1 MiB window before the kernel refuses the memslot; a hundred of them is 100 MiB, two
/// orders of magnitude above the noise of an allocator warming up. The open-descriptor
/// count is the second half of the same property: with `shareable_ram`, every attempt also
/// creates a `memfd` before the ioctl refuses, and a leaked descriptor moves no counter
/// either.
///
/// ## ★★ Why the measurement runs in a CHILD PROCESS, and why that is not optional
///
/// `VmSize` is a property of the **process**, and this file's other eighteen tests share
/// it. libtest runs them on `available_parallelism()` threads, so on a big box a sibling's
/// 2 MiB thread stack — or, far worse, a 64 MiB glibc per-thread malloc arena, which is
/// created on allocator *contention* and is therefore a direct function of how many
/// siblings overlap — lands between `before` and `after` and is charged here. Measured at
/// `634b253` on a 25-core box: **26 failures in 30 runs**, with growths quantised to
/// multiples of 64 MiB (65 580, 131 144, 200 812, 471 208 KiB), and **0 in 30** with
/// `--test-threads=1`. The same commit was green on a 4-core box. Note that most of those
/// numbers are *larger than the 100 MiB the leak under test can even produce*: a reading
/// above the theoretical maximum of the bug is the tell that the instrument is measuring
/// somebody else.
///
/// Three fixes were rejected before this one:
///
/// - **`--test-threads=1`.** A test whose correctness depends on how the suite happens to
///   be invoked breaks the first day somebody runs it normally, and CI runs it normally.
/// - **A process-local substitute for the reading.** There is none: the address space *is*
///   process-global. `/proc/self/maps` totals have exactly the same contamination.
/// - **Naming our own mapping** — record the window's host VA and assert that range is
///   absent from `/proc/self/maps` afterwards. This races the siblings from the *other*
///   side: a just-freed hole is precisely what the next `mmap` in any thread reuses, so a
///   correct release would intermittently read as a leak.
///
/// What is left is to give the measurement a process in which it **is** local. The test
/// re-execs its own binary with `--exact … --test-threads=1` and `KAYFABE_ADDRSPACE_CHILD`
/// set; the child, where nothing else is running, measures and prints
/// [`ADDRSPACE_SENTINEL`], and the parent asserts the exact exit code **and** the sentinel
/// — because a libtest filter that matches nothing also exits 0, which would make this a
/// green instrument over a measurement nobody took. The result is immune to core count, to
/// the parent's `--test-threads`, and to every sibling test, by construction rather than by
/// convention.
#[test]
fn a_repeated_partial_failure_returns_the_host_address_space() {
    kayfabe_linux_raw::require_kvm!("a_repeated_partial_failure_returns_the_host_address_space");
    if std::env::var_os(ADDRSPACE_CHILD).is_some() {
        measure_the_partial_failure_path_alone();
        return;
    }
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let out = std::process::Command::new(&exe)
        .args([
            "--exact",
            "a_repeated_partial_failure_returns_the_host_address_space",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(ADDRSPACE_CHILD, "1")
        .output()
        .unwrap_or_else(|e| panic!("re-exec {} for the lone measurement: {e}", exe.display()));
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "★ the lone-process measurement failed. Its whole output follows, and the \
         `{ADDRSPACE_SENTINEL}` line carries the numbers:\n{said}"
    );
    assert!(
        said.contains(ADDRSPACE_SENTINEL),
        "★ NON-VACUITY: the child exited 0 without ever printing `{ADDRSPACE_SENTINEL}`. \
         A libtest filter that matches NOTHING exits 0 too, so this assertion is the only \
         thing between a green suite and a measurement that never ran — rename the test \
         and this is what catches the stale filter:\n{said}"
    );
}

/// The environment variable that tells a re-exec of this binary it is the measuring child.
const ADDRSPACE_CHILD: &str = "KAYFABE_ADDRSPACE_CHILD";

/// The child's proof-of-work line. Its absence is a vacuous pass; see the test above.
const ADDRSPACE_SENTINEL: &str = "KAYFABE-ADDRSPACE-MEASURED";

/// ★ The measurement, run **alone** in a child process — the reason it is a separate
/// function is that its two readings (`VmSize`, open descriptors) are process-global and
/// are only local here.
fn measure_the_partial_failure_path_alone() {
    let refused = Err(VmmError::HostRefused {
        what: "a memslot install",
        errno: Some(libc::EEXIST),
    });
    let m = machine();
    let p = page();
    let win = 256 * p.bytes();
    m.install_ram_window(GPA_RAM, win)
        .expect("the first window");
    // Warm up, so the baseline is not measuring the first refusal's own allocations.
    for _ in 0..4 {
        assert_eq!(
            m.install_ram_window(GPA_RAM + 8 * p.bytes(), win),
            refused.clone()
        );
    }
    let (before, fds_before) = (vm_size_kib(), open_descriptors());
    for _ in 0..100 {
        assert_eq!(
            m.install_ram_window(GPA_RAM + 8 * p.bytes(), win),
            refused.clone()
        );
    }
    let (after, fds_after) = (vm_size_kib(), open_descriptors());
    let leaked = after.saturating_sub(before);
    let fds_leaked = fds_after.saturating_sub(fds_before);
    println!("{ADDRSPACE_SENTINEL} vmsize_growth_kib={leaked} fd_growth={fds_leaked}");
    // Alone in its own process both readings measure **exactly 0**, over 60 runs including
    // 20 taken while five copies of the whole binary hammered the box. 8 MiB is
    // nevertheless the right slack rather than something tighter: the two noise sources
    // that exist at all are a 2 MiB thread stack and a 64 MiB glibc arena, so any bound
    // between 2 MiB and 64 MiB is exactly as robust as any other — and 8 MiB is a
    // twelfth of the leak it is looking for, so it still bites if only eight of the
    // hundred attempts leak rather than all of them.
    assert!(
        leaked < 8 * 1024,
        "★ a hundred refused installs grew this process's address space by {leaked} KiB. \
         Each one maps a {win_kib} KiB window BEFORE the kernel refuses the memslot; if \
         the failure path forgets to release it, the leak is ~{expect} KiB and the \
         conservation ledger — which is only incremented on SUCCESS — says nothing at all",
        win_kib = win / 1024,
        expect = 100 * win / 1024,
    );
    assert!(
        fds_leaked < 8,
        "★ and the DESCRIPTORS too: a hundred refused installs left {fds_leaked} open. \
         Each attempt creates a shareable-RAM `memfd` before the ioctl refuses, so a \
         failure path that releases the mapping but not the backing leaks a descriptor \
         per attempt — invisible to `VmSize` and invisible to the ledger"
    );
    assert_eq!(
        m.audit().host_refusals,
        104,
        "★ NON-VACUITY: every attempt really was refused by the host"
    );
}

/// `VmSize` in KiB, from `/proc/self/status`.
fn vm_size_kib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").expect("procfs");
    s.lines()
        .find_map(|l| l.strip_prefix("VmSize:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .expect("VmSize is always present on Linux")
}

/// How many descriptors this process holds open, from `/proc/self/fd`.
fn open_descriptors() -> u64 {
    std::fs::read_dir("/proc/self/fd").expect("procfs").count() as u64
}

/// ★ `ENOMEM` from the kernel: a window the address space cannot hold. The `mmap` fails
/// **before** any memslot exists, which is the other partial-failure shape.
#[test]
fn a_window_the_address_space_cannot_hold_is_the_kernels_enomem() {
    kayfabe_linux_raw::require_kvm!("a_window_the_address_space_cannot_hold_is_the_kernels_enomem");
    let m = machine();
    let p = page();
    let absurd = (1u64 << 62) & !p.mask();
    assert_eq!(
        m.install_ram_window(GPA_RAM, absurd),
        Err(VmmError::HostRefused {
            what: "a window mapping",
            errno: Some(libc::ENOMEM),
        })
    );
    let a = m.audit();
    assert_eq!(
        (a.live_windows, a.live_memslots, a.window_bytes),
        (0, 0, 0),
        "an mmap that failed must leave no window and no declaration"
    );
    assert_eq!(a.host_refusals, 1, "★ NON-VACUITY: the refusal was counted");
}

/// ★★★ The **memslot ceiling** is a real, kernel-imposed number — and what it binds on is
/// numbers **live at once**, not numbers ever issued.
///
/// # What this test used to assert, and why that was the defect
///
/// It used to walk `ceiling` **install/remove** cycles and assert the next install
/// refused, on the stated ground that slot numbers *"are deliberately not recycled — a
/// recycled slot number is indistinguishable from a stale one in a kernel log"*. So the
/// green test was pinning, as a requirement, exactly the behaviour the sibling QEMU
/// adapter's own allocator docs record as the C artifact's **measured** failure:
/// `C: nvkvm_mmap_host.c:382-389`, a never-recycling allocator that *"exhausted the pool
/// after a few CUDA processes"*. Neither doc cited the other. See [`slotnum`] for the
/// adjudication.
///
/// So the test is split in two, and neither half is weaker than the old one: churn must
/// **not** exhaust (the C's datum, on the real plane), and the ceiling must still refuse
/// by name when that many windows are genuinely live at once.
#[test]
fn the_memslot_ceiling_is_a_real_number_and_the_refusal_names_it() {
    kayfabe_linux_raw::require_kvm!(
        "the_memslot_ceiling_is_a_real_number_and_the_refusal_names_it"
    );
    let m = machine();
    let ceiling = m.memslot_ceiling();
    assert!(
        ceiling >= 32,
        "the kernel reported {ceiling} memslots, which is not a real ceiling — the \
         capability query is not reaching KVM"
    );
    let p = page();

    // ---- half 1: CHURN, twice the ceiling, on real memslots. -----------------------
    // With the allocator this replaced, iteration `ceiling` of this loop refuses.
    for i in 0..ceiling * 2 {
        let r = m
            .install_ram_window(GPA_RAM + u64::from(i % 4) * p.bytes() * 2, p.bytes())
            .unwrap_or_else(|e| {
                panic!(
                    "iteration {i} of a workload that holds ONE window at a time exhausted \
                     the kernel's memslot pool ({e:?}) — this is the C's measured failure, \
                     reproduced"
                )
            });
        m.remove_window(r).expect("remove");
    }
    let a = m.audit();
    assert_eq!(a.live_memslots, 0, "the churn left nothing live");
    assert_eq!(
        a.memslot_installs,
        u64::from(ceiling) * 2,
        "★ NON-VACUITY: twice the ceiling's worth of REAL installs happened"
    );
    assert_eq!(
        a.slot_numbers_recycled,
        u64::from(ceiling) * 2 - 1,
        "★ NON-VACUITY: the free list — not a bigger pool — is what served them; every \
         install after the first re-issued a number"
    );

    // ---- half 2: the ceiling still binds, on numbers live SIMULTANEOUSLY. ----------
    // Kept honest by holding the windows rather than by counting issues. `shareable_ram`
    // is off for this machine only: a shareable backing is one memfd per window, and
    // `ceiling`-many live descriptors is a file-descriptor limit masquerading as a
    // memslot limit.
    let m = KvmMachine::realize(MachineConfig {
        shareable_ram: false,
        bars: vec![BarPlacement {
            bar: BarId::Bar0,
            base: GPA_BAR0,
            len: BAR0_LEN,
        }],
    })
    .expect("/dev/kvm must be present and permitted");
    let mut held = Vec::with_capacity(ceiling as usize);
    for i in 0..ceiling {
        held.push(
            m.install_ram_window(GPA_RAM + u64::from(i) * p.bytes() * 2, p.bytes())
                .unwrap_or_else(|e| panic!("window {i} of {ceiling} must fit ({e:?})")),
        );
    }
    // ★★ The probe GPA is derived from this test's OWN layout — the page just past the
    // last window installed above — and deliberately NOT a far-away constant.
    //
    // It used to be `0x9000_0000_0000`, and that made the test **CPU-dependent**:
    // `KVM_SET_USER_MEMORY_REGION` refuses (EINVAL) any GPA above the *host CPU's*
    // physical-address width. Measured 2026-07-30 with the same probe on both boxes —
    // AMD EPYC 7543 (48 phys bits) accepts it, Intel Xeon E5-2697A v4 (46 phys bits)
    // refuses it — so the test passed on one machine and failed on the other for a
    // reason with nothing to do with memslots. Deriving the address means there is no
    // width to be wrong about: this GPA sits a few hundred MiB up, legal on any host.
    let probe_gpa = GPA_RAM + u64::from(ceiling) * p.bytes() * 2;
    assert_eq!(
        m.install_ram_window(probe_gpa, p.bytes()),
        Err(VmmError::Unsupported(
            kayfabe_vmm_kvm::slotnum::MEMSLOT_CEILING_REACHED
        )),
        "with every number live at once the ceiling must refuse, by a name that says \
         which resource ran out"
    );
    assert_eq!(
        m.audit().live_memslots,
        u64::from(ceiling),
        "★ NON-VACUITY: they really were all live"
    );
    // And giving exactly one back makes exactly one available — the property that
    // distinguishes a recycling allocator from one that merely has a large pool.
    m.remove_window(held.pop().expect("one to give back"))
        .expect("remove");
    // ★ The message names what this call DID, not what its failure would prove. The
    // previous text ("the number that just came back") named the *hypothesis*, so the
    // unrelated EINVAL above surfaced as apparent evidence that the recycling allocator
    // had broken — a failure that misattributes itself costs more than the bug.
    let r = m
        .install_ram_window(probe_gpa, p.bytes())
        .expect("installing one window after freeing one, with the ceiling otherwise full");
    m.remove_window(r).expect("remove");
}

/// ★★★ **The mean shape of the recycling change**: many windows live at once, of mixed
/// arity (a read-native overlay is three memslots, an ordinary window one), churned in an
/// order that is neither FIFO nor LIFO, with a registered trap standing over it the whole
/// time — and after every step the plane must still agree with itself.
///
/// The property recycling puts at risk is not exhaustion, it is **aliasing**: a number
/// handed back before its slot was cleared gets re-issued and the next install is a silent
/// REPLACE. The kernel reports nothing, `install` returns `Ok`, every counter balances.
/// `assert_map_matches_the_kernel`'s distinctness clause is the only observable, so this
/// drives the state where duplicates would appear and then asks.
#[test]
fn recycled_memslot_numbers_never_alias_across_a_mean_churn() {
    kayfabe_linux_raw::require_kvm!("recycled_memslot_numbers_never_alias_across_a_mean_churn");
    let m = machine();
    let p = page();
    let b = p.bytes();
    let mut v = m.vmm();

    // A read-native overlay over the BAR's head — two memslots, `[trapped page RO][tail
    // RW]` — and a trap registered over its read-only span. Both must survive every churn
    // below: the trap cross-check runs on every `assert_map_matches_the_kernel` call in
    // the loop.
    let backing = m.register_backing(2 * b).expect("backing");
    v.map_read_native(GPA_BAR0, 2 * b, backing, Some(GPA_BAR0..GPA_BAR0 + b))
        .expect("the overlay installs");
    v.set_trap(BarId::Bar0, 0..b, TrapMode::WriteOnly)
        .expect("a write-only trap over the read-only span");

    // Now churn ordinary windows around it, in a rotation that frees the OLDEST while the
    // newest is still live, so the free list is never empty and never drained in issue
    // order.
    let mut live: Vec<kayfabe_vmm::RamRegionId> = Vec::new();
    for i in 0..200u64 {
        live.push(
            m.install_ram_window(GPA_RAM + (i % 16) * 2 * b, b)
                .unwrap_or_else(|e| panic!("window {i} must install ({e:?})")),
        );
        if live.len() > 8 {
            m.remove_window(live.remove(0)).expect("the oldest goes");
        }
        // The whole plane, every iteration: view vs installer, trap vs tiering, and no two
        // live memslots sharing a number.
        let checked = m.assert_map_matches_the_kernel();
        assert_eq!(
            checked,
            live.len() + 1,
            "★ NON-VACUITY: the check saw every live window (the {} ordinary ones plus the \
             overlay), not an empty map",
            live.len()
        );
    }
    // ★★★ And the shape that makes the distinctness clause reachable at all: a second
    // window over a guest-physical range a live one already covers. With correct
    // allocation the two carry DIFFERENT numbers, so the kernel sees an overlapping range
    // and refuses with `EEXIST`. Hand out a number that is already live and the very same
    // call becomes a silent in-place REPLACE (measured in the QEMU adapter's
    // `kvm_differential`: same number, same base, same size — `Ok`, and the kernel says
    // nothing) — after which two live windows in our books point at one kernel slot, and
    // the first window's mapping is reachable by nobody while its handle still promises it
    // is installed.
    let gpa_of_a_live_one = GPA_RAM + (199 % 16) * 2 * b;
    let collided = m.install_ram_window(gpa_of_a_live_one, b);
    // ★ The plane is interrogated FIRST, and on the outcome that actually happened — so
    // that a duplicate number is reported as a duplicate number rather than as a surprising
    // `Ok` from an install. Asserting the refusal first would make the sharper failure
    // unreachable: the weaker assertion would always fire before it.
    assert_eq!(
        m.assert_map_matches_the_kernel(),
        live.len() + 1 + usize::from(collided.is_ok()),
        "★ whatever the kernel did, no two live windows may share a kernel memslot number"
    );
    assert!(
        matches!(collided, Err(VmmError::HostRefused { .. })),
        "and what it must have done is REFUSE: distinct numbers over an overlapping \
         guest-physical range is the kernel's own EEXIST ({collided:?})"
    );
    if let Ok(r) = collided {
        m.remove_window(r).expect("unwind the surprise");
    }

    for r in live {
        m.remove_window(r).expect("teardown");
    }
    let a = m.audit();
    assert_eq!(a.live_memslots, 2, "only the overlay's two spans remain");
    assert!(
        a.slot_numbers_recycled >= 190,
        "★ NON-VACUITY: this churn must actually have RE-ISSUED numbers — with a \
         never-recycling allocator it is 0 and every other assertion here still passes \
         (got {})",
        a.slot_numbers_recycled
    );
    assert!(
        a.peak_memslots >= 10,
        "★ NON-VACUITY: at least ten slots really were live at once — eight ordinary \
         windows plus the overlay's two spans (got {})",
        a.peak_memslots
    );
}

/// ★ Alignment and emptiness are refused **before** the syscall, by name.
#[test]
fn a_misaligned_or_empty_window_is_refused_before_any_syscall() {
    kayfabe_linux_raw::require_kvm!("a_misaligned_or_empty_window_is_refused_before_any_syscall");
    let m = machine();
    let p = page();
    let expected = Err(VmmError::Unsupported(
        "a window whose base or length is not a whole number of host pages",
    ));
    assert_eq!(m.install_ram_window(GPA_RAM + 8, p.bytes()), expected);
    assert_eq!(m.install_ram_window(GPA_RAM, p.bytes() + 1), expected);
    assert_eq!(m.install_ram_window(GPA_RAM, 0), expected);
    assert_eq!(
        m.audit().memslot_installs,
        0,
        "not one ioctl may have been issued"
    );
}

// =================================================================================
// ★★★ R1 — the invariant this stage exists to test
// =================================================================================

/// ★★★ **The R1 witness.** Every syscall-shaped method must run with ZERO ranked locks,
/// and the in-lock-legal accessors must really have been entered WITH one — otherwise the
/// suite is green about a path nobody took.
///
/// Both halves are load-bearing, and the second is the one a naive version omits.
#[test]
fn no_syscall_shaped_method_ever_runs_with_a_ranked_lock_held() {
    kayfabe_linux_raw::require_kvm!("no_syscall_shaped_method_ever_runs_with_a_ranked_lock_held");
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let backing = m.register_backing(p.bytes()).expect("backing");

    // --- syscalls, all lock-free (the ordinary path)
    let region = m
        .install_ram_window(GPA_RAM, 4 * p.bytes())
        .expect("window");
    let slot = v
        .map_guest(GPA_RAM, p.bytes(), backing, Prot::ReadWrite)
        .expect("place");
    v.export_ram(None).expect("export");
    v.unmap_guest(slot).expect("restore");

    // --- an in-lock-legal accessor, entered WITH a ranked lock, exactly as
    //     `SharedDevice::parse_pushbuffer`'s route phase enters it.
    lockwitness::note_acquired(0);
    let mut buf = [0u8; 64];
    let in_lock = v.gpa_read(GPA_RAM, &mut buf);
    lockwitness::note_released(0);
    assert_eq!(
        in_lock,
        Ok(()),
        "gpa_read is in-lock LEGAL and must be served"
    );

    // --- and one lock-free, so the span has two ends.
    v.gpa_read(GPA_RAM, &mut buf).expect("lock-free read");

    let a = m.audit();
    assert_eq!(
        a.syscall_ranked_depth,
        (0, 0),
        "★ R1: every syscall-shaped Vmm method ran with zero ranked locks held. A `min` \
         of u32::MAX would mean NO syscall ran at all, which is why the pair is asserted \
         and not just the max"
    );
    assert_eq!(
        a.accessor_ranked_depth,
        (0, 1),
        "★ and the in-lock-legal accessor really was entered with a ranked lock held \
         (max 1) AND lock-free (min 0). Without both ends this whole file could be green \
         about a path the harness never took"
    );
    assert_eq!(
        a.copy_leaf_depth_max, 0,
        "★★ THE ADAPTER HALF OF R1 (§12.43 residual 2): the memcpy runs OUTSIDE the view \
         lock, held alive by an Arc. A copy under the lock would serialise every \
         guest-memory read in the machine against every other"
    );
    assert!(
        a.view_leaf_depth_max >= 1,
        "★ NON-VACUITY for the line above: the view lock must actually have been taken. \
         With no lock at all, `copy_leaf_depth_max == 0` is true and means nothing"
    );
    m.remove_window(region).expect("remove");
}

/// ★ **The witnesses' own non-vacuity, asserted directly.** A machine on which nothing has
/// happened must report *"never observed"* and not *"observed zero"* — otherwise every
/// span assertion in this file has a lower bound that equals its own success value, which
/// is the exact defect `l1_concurrency.md` §12.43's N12 found (`Arc<AtomicU32>::default()`
/// is `0`). A bite-check on the hand-written `Default` did not bite until this test
/// existed.
#[test]
fn a_fresh_machine_reports_never_observed_and_not_observed_zero() {
    kayfabe_linux_raw::require_kvm!("a_fresh_machine_reports_never_observed_and_not_observed_zero");
    let m = machine();
    let a = m.audit();
    assert_eq!(
        (a.accessor_ranked_depth, a.syscall_ranked_depth),
        ((u32::MAX, 0), (u32::MAX, 0)),
        "★ the minima must start at u32::MAX. With a derived Default they start at 0, and \
         `syscall_ranked_depth == (0, 0)` — R1's headline assertion — becomes true of a \
         machine that never made a syscall at all"
    );
    assert_eq!(
        (a.copy_leaf_depth_max, a.view_leaf_depth_max),
        (0, 0),
        "and the leaf witnesses start at zero, so `>= 1` really is an observation"
    );
}

/// ★★ The adapter's own witness, in the polarity that matters: a syscall attempted while
/// the adapter holds one of its own locks must be **loud**, even though no ranked lock is
/// held and `lockwitness` is therefore silent.
#[test]
fn a_syscall_under_the_adapters_own_lock_is_loud_even_though_no_rank_is_held() {
    kayfabe_linux_raw::require_kvm!(
        "a_syscall_under_the_adapters_own_lock_is_loud_even_though_no_rank_is_held"
    );
    let m = machine();
    let p = page();
    let held = leaf::Held::enter();
    assert_eq!(
        lockwitness::held_depth(),
        0,
        "the RANKED witness sees nothing here — which is exactly the blind spot"
    );
    let boom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        m.install_ram_window(GPA_RAM, p.bytes())
    }));
    drop(held);
    let err = boom.expect_err("a syscall under an adapter lock must panic");
    let msg = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        msg.contains("R1 violation, adapter half"),
        "the panic must name R1's adapter half, not merely fail: {msg}"
    );
}

// =================================================================================
// The remaining capability groups
// =================================================================================

/// ★ §6.7 item 4: KVM's read-only flag is a **slot** property, so per-object read-only
/// protection inside a shared read-write window is refused rather than approximated.
#[test]
fn per_object_read_only_protection_is_refused_because_protection_is_a_window_property() {
    kayfabe_linux_raw::require_kvm!(
        "per_object_read_only_protection_is_refused_because_protection_is_a_window_property"
    );
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, 2 * p.bytes())
        .expect("window");
    let backing = m.register_backing(p.bytes()).expect("backing");
    assert_eq!(
        v.map_guest(GPA_RAM, p.bytes(), backing, Prot::ReadOnly),
        Err(VmmError::Unsupported(
            "per-object read-only protection inside a read-write window — \
                     protection is a WINDOW property (§6.7 item 4); place the object in \
                     a read-only window instead"
        )),
        "minting a slot to make one page read-only is a DELETE+ADD (two grace periods) \
         to protect one object, and it revokes access for every proc sharing the window"
    );
    assert_eq!(
        m.audit().placements_made,
        0,
        "and nothing was placed on the way to the refusal"
    );
}

/// ★ The read-native overlay is a **read-only memslot** — the rom-device pattern, not an
/// emulation of it — and the write-trap sub-range is rounded outward to whole host pages.
#[test]
fn a_read_native_overlay_installs_a_read_only_slot_over_the_rounded_write_trap() {
    kayfabe_linux_raw::require_kvm!(
        "a_read_native_overlay_installs_a_read_only_slot_over_the_rounded_write_trap"
    );
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let backing = m.register_backing(4 * p.bytes()).expect("backing");
    // ★ Over the BAR, not over anonymous guest RAM: since #87 a write-trap span must have
    // a device behind it, because the store it traps has to be dispatchable to one.
    let trap = (GPA_BAR0 + p.bytes() + 8)..(GPA_BAR0 + p.bytes() + 9);
    v.map_read_native(GPA_BAR0, 4 * p.bytes(), backing, Some(trap))
        .expect("a read-native overlay installs");
    let a = m.audit();
    assert_eq!(
        (a.live_windows, a.live_memslots),
        (1, 3),
        "one window, three slots: [head RW][the trapped page RO][tail RW]. A single-byte \
         request became a whole HOST page, which on an arm64 host is 16 or 64 KiB — \
         correct, and quietly slower"
    );
    // Reads are still served natively through our own accessor.
    let mut got = [0xFFu8; 8];
    v.gpa_read(GPA_BAR0 + p.bytes(), &mut got)
        .expect("a read-native range is RAM to us as well");
    assert_eq!(got, [0u8; 8]);

    assert_eq!(
        v.map_read_native(GPA_BAR0 + 8 * p.bytes(), p.bytes(), backing, Some(0..1)),
        Err(VmmError::Unsupported(
            "a write-trap sub-range that is not inside the range it overlays"
        )),
        "a trap outside the range it overlays is a refusal, not a silent clamp"
    );
}

/// ★★ **#87: a write-trap span with no device behind it is REFUSED**, because the store it
/// traps would have nowhere to go.
///
/// The read-only memslot really does send the guest's store out to userspace. But the exit
/// carries a guest-physical address and nothing else, so the dispatcher can only route it
/// by finding it inside a realized BAR. Outside one it classifies `Unclaimed` and the
/// guest's write is **dropped** — a lost store, with no error anywhere. The only place a
/// caller can still be told is here.
#[test]
fn a_write_trap_span_no_realized_bar_covers_is_refused_because_its_stores_would_be_dropped() {
    kayfabe_linux_raw::require_kvm!(
        "a_write_trap_span_no_realized_bar_covers_is_refused_because_its_stores_would_be_dropped"
    );
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let backing = m.register_backing(2 * p.bytes()).expect("backing");
    let refusal = Err(VmmError::Unsupported(
        "a read-native write-trap span (rounded to whole host pages) that is not inside \
         any realized BAR — the guest's store would exit, classify as unclaimed, and be \
         dropped",
    ));

    // (a) Nowhere near a BAR.
    assert_eq!(
        v.map_read_native(
            GPA_RAM,
            2 * p.bytes(),
            backing,
            Some(GPA_RAM..GPA_RAM + p.bytes())
        ),
        refusal,
        "an overlay over plain guest RAM traps stores nobody can deliver"
    );
    assert_eq!(
        (m.audit().live_windows, m.audit().live_memslots),
        (0, 0),
        "★ and the refusal happened BEFORE any syscall — nothing was mapped on the way out"
    );

    // (b) ★ The mean one: the window overlaps the BAR, but the trap span falls off the
    // END of it. Checking "the window is in a BAR" instead of "the trap span is" would
    // pass this and drop every store to the last page.
    assert_eq!(
        v.map_read_native(
            GPA_BAR0 + BAR0_LEN - p.bytes(),
            2 * p.bytes(),
            backing,
            Some(GPA_BAR0 + BAR0_LEN..GPA_BAR0 + BAR0_LEN + p.bytes())
        ),
        refusal,
        "★ the span, not the window, is what must be covered — the trapped page here is \
         one page PAST the BAR's end"
    );

    // (c) And the same shape wholly inside the BAR installs, so (a) and (b) are refusals
    // about coverage and not about read-native overlays in general.
    v.map_read_native(
        GPA_BAR0,
        2 * p.bytes(),
        backing,
        Some(GPA_BAR0..GPA_BAR0 + p.bytes()),
    )
    .expect("★ NON-VACUITY: the very same call inside the BAR is accepted");
    assert_eq!(
        (m.audit().live_windows, m.audit().live_memslots),
        (1, 2),
        "one window, two slots: [the trapped page RO][tail RW]"
    );
}

/// ★★ **#87's other half at the registration seam**: `set_trap(WriteOnly)` used to accept
/// any range that merely *resolved*, including an ordinary read-write window — over which
/// a write does not exit at all. A registration that reads as protection and is none.
#[test]
fn a_write_only_trap_is_refused_over_a_read_write_window_and_accepted_over_a_read_only_one() {
    kayfabe_linux_raw::require_kvm!(
        "a_write_only_trap_is_refused_over_a_read_write_window_and_accepted_over_a_read_only_one"
    );
    let m = machine();
    let p = page();
    let mut v = m.vmm();

    // An ORDINARY window over the BAR: it resolves, so the old check passed. Its writes
    // land in RAM.
    let plain = m
        .install_ram_window(GPA_BAR0, p.bytes())
        .expect("a read-write window over the BAR's first page");
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..p.bytes(), TrapMode::WriteOnly),
        Err(VmmError::Unsupported(
            "a write-only trap over a range no read-native overlay traps — reads are \
             served from RAM only if a memslot exists, and writes exit only if that \
             memslot is READ-ONLY; install the overlay with `map_read_native` first"
        )),
        "★ resolving is NOT enough: over a read-write memslot the guest's store never \
         leaves the guest, and the trap would exist only in our bookkeeping"
    );
    m.remove_window(plain).expect("the plain window goes");

    // The same range, this time under a read-native overlay's read-only span.
    let backing = m.register_backing(2 * p.bytes()).expect("backing");
    let slot = v
        .map_read_native(
            GPA_BAR0,
            2 * p.bytes(),
            backing,
            Some(GPA_BAR0..GPA_BAR0 + p.bytes()),
        )
        .expect("the overlay installs");
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..p.bytes(), TrapMode::WriteOnly),
        Ok(()),
        "★ NON-VACUITY: with the read-only slot actually there, the registration is right"
    );
    assert_eq!(
        m.assert_map_matches_the_kernel(),
        1,
        "and the registered trap cross-checks against the live plane"
    );

    // ★ And the assertion is not a no-op: pull the overlay out from under the registered
    // trap and the cross-check must SEE it.
    v.unmap_guest(slot).expect("the overlay goes");
    let seen = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        m.assert_map_matches_the_kernel()
    }));
    let err = seen.expect_err(
        "★ a write-only trap whose read-only span was removed must FAIL the cross-check — \
         until #87 `Installer::traps` was never read by anything, so nothing could notice",
    );
    let msg = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default();
    assert!(
        msg.contains("no read-native overlay's read-only span covers it"),
        "and it must fail for THAT reason, not merely fail: {msg}"
    );
}

/// ★ A trap over a range a live memslot serves is refused: the guest's access would never
/// leave the guest, so the trap would exist only in our bookkeeping.
#[test]
fn a_read_write_trap_over_a_live_memslot_is_refused() {
    kayfabe_linux_raw::require_kvm!("a_read_write_trap_over_a_live_memslot_is_refused");
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..p.bytes(), TrapMode::ReadWrite),
        Ok(()),
        "the BAR has no memslot, so a trap over it is exactly right"
    );
    // Now put RAM under part of the BAR — the shape a guest produces by reprogramming a
    // BAR over RAM — and the trap must be refused.
    m.install_ram_window(GPA_BAR0, p.bytes())
        .expect("a window over the BAR's base");
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..p.bytes(), TrapMode::ReadWrite),
        Err(VmmError::Unsupported(
            "a read-write trap over a range a live memslot already serves — \
                         the guest's access would never leave the guest"
        ))
    );
    assert_eq!(
        v.set_trap(BarId::Bar1, 0..p.bytes(), TrapMode::ReadWrite),
        Err(VmmError::Unsupported(
            "a BAR this machine was not realized with"
        ))
    );
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..(BAR0_LEN + 1), TrapMode::ReadWrite),
        Err(VmmError::Unsupported("a trap range outside its BAR"))
    );
}

/// ★ §4.4.1's deployment fact: a VM created without a shareable backing refuses the
/// **first** export, loudly, rather than handing back copy-on-write pages that would make
/// an isolate's completions invisible to the guest.
#[test]
fn a_machine_without_a_shareable_backing_refuses_the_first_export() {
    kayfabe_linux_raw::require_kvm!(
        "a_machine_without_a_shareable_backing_refuses_the_first_export"
    );
    let m = KvmMachine::realize(MachineConfig {
        shareable_ram: false,
        bars: Vec::new(),
    })
    .expect("realize");
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, p.bytes()).expect("window");
    // Everything else still works — which is exactly why the refusal has to be loud.
    v.gpa_write(GPA_RAM, &[7; 4]).expect("guest memory works");
    assert_eq!(
        v.export_ram(None),
        Err(VmmError::Unsupported(
            "guest RAM was not created with a shareable backing; an isolate cannot map it"
        ))
    );

    let shared = machine();
    shared
        .install_ram_window(GPA_RAM, p.bytes())
        .expect("window");
    let h = shared.vmm().export_ram(None).expect("a shareable export");
    assert_eq!(h.covers, None);
    assert_eq!(
        shared
            .vmm()
            .export_ram(Some(GPA_RAM..(GPA_RAM + p.bytes()))),
        Ok(kayfabe_vmm::RamHandle {
            token: kayfabe_vmm::RAM_EXPORT_TOKEN_TAG | 1,
            covers: Some(GPA_RAM..(GPA_RAM + p.bytes()))
        }),
        "a per-slice export is a distinct handle — least-privilege sharing (§4.3.4)"
    );
    assert_ne!(
        h.token & kayfabe_vmm::RAM_EXPORT_TOKEN_TAG,
        0,
        "★★★ every guest-RAM token carries the tag; an untagged one is a valid HostRegion id"
    );
    assert_eq!(
        shared.vmm().export_ram(Some(0xDEAD_0000..0xDEAD_1000)),
        Err(VmmError::Unsupported(
            "no shareable window covers the requested slice"
        ))
    );
}

/// ★ `raise_irq` is the one in-lock-legal syscall, and it really is one: a write to a
/// real notify descriptor, permitted under the ranks it declares.
#[test]
fn raise_irq_is_a_real_descriptor_write_and_is_legal_under_the_ranks_it_declares() {
    kayfabe_linux_raw::require_kvm!(
        "raise_irq_is_a_real_descriptor_write_and_is_legal_under_the_ranks_it_declares"
    );
    let m = machine();
    let mut v = m.vmm();
    lockwitness::note_acquired(1);
    let under_lock = v.raise_irq(IrqSpec::Msix(0));
    lockwitness::note_released(1);
    assert_eq!(
        under_lock,
        Ok(()),
        "§6.1's single named exception — an irqfd-shaped edge under the proc lock"
    );
    v.raise_irq(IrqSpec::Msix(0)).expect("and lock-free");
    assert_eq!(
        m.drain_irqs(),
        Ok(2),
        "two real edges accumulated on a real descriptor — a counter, not a Vec of enums"
    );
    assert_eq!(
        v.raise_irq(IrqSpec::IntxLevel(true)),
        Err(VmmError::Unsupported(
            "legacy INTx needs an interrupt controller this harness does not create"
        )),
        "backend-conditional by the trait's own rustdoc: a REFUSAL, never a silently \
         dropped injection"
    );
}

/// ★ The deadline queue is the port's, shared with the mock (§6.4) — so two backends
/// cannot drift on timer ordering.
#[test]
fn deferred_events_come_back_in_deadline_then_insertion_order() {
    kayfabe_linux_raw::require_kvm!("deferred_events_come_back_in_deadline_then_insertion_order");
    let m = machine();
    let mut v = m.vmm();
    v.defer(
        Duration::from_millis(2),
        CoreEvent::Deferred(CoreEventKind::DeferredReap),
    );
    v.defer(
        Duration::from_millis(1),
        CoreEvent::Deferred(CoreEventKind::PollKickBudget),
    );
    v.defer(
        Duration::from_millis(1),
        CoreEvent::Deferred(CoreEventKind::CompletionRedeliver(kayfabe_vmm::GpuId::ZERO)),
    );
    assert_eq!(m.advance(Duration::from_micros(500)), vec![]);
    assert_eq!(
        m.advance(Duration::from_micros(500)),
        vec![
            CoreEvent::Deferred(CoreEventKind::PollKickBudget),
            CoreEvent::Deferred(CoreEventKind::CompletionRedeliver(kayfabe_vmm::GpuId::ZERO)),
        ],
        "same deadline: INSERTION order, not the payload's own Ord — otherwise delivery \
         order would depend on the event value, deterministically and wrongly"
    );
    assert_eq!(
        m.advance(Duration::from_millis(1)),
        vec![CoreEvent::Deferred(CoreEventKind::DeferredReap)]
    );
    assert_eq!(
        v.now(),
        kayfabe_util::Instant::ZERO.advanced(Duration::from_millis(2))
    );
}

/// ★ A publication naming a backing this backend never minted is refused, not served
/// from whatever happened to be at that index.
#[test]
fn a_backing_id_this_backend_never_minted_is_refused() {
    kayfabe_linux_raw::require_kvm!("a_backing_id_this_backend_never_minted_is_refused");
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, p.bytes()).expect("window");
    assert_eq!(
        v.map_guest(
            GPA_RAM,
            p.bytes(),
            HostRegion { id: 999, offset: 0 },
            Prot::ReadWrite
        ),
        Err(VmmError::Unsupported(
            "a host backing id this backend never minted"
        ))
    );
    assert_eq!(
        v.map_guest(
            0x5000_0000,
            p.bytes(),
            m.register_backing(p.bytes()).expect("backing"),
            Prot::ReadWrite
        ),
        Err(VmmError::BadGpa { gpa: 0x5000_0000 }),
        "and a publication outside every installed window is a BadGpa — the fine tier \
         cannot conjure a window, which is §6.7's rule stated as a refusal"
    );
}
