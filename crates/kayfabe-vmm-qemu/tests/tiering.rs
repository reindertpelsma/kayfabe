//! ★★★ The three tiers — **new construction, and gated as such**.
//!
//! `host_execution_plane.md` §1.5's last box: Mode 2 in the C has **never had a memslot**
//! (`C: nvkvm_gpu_emul.c:9743-9779` is all trapping MMIO and
//! `KVM_SET_USER_MEMORY_REGION` appears nowhere in that file), and the Mode-1 window that
//! does have one has exactly one, read-write, forever — `readonly` is a dead parameter
//! there. So the C is precedent for **reservation + one read-write slot** and for nothing
//! else, and everything in this file has to earn its own green gate.
//!
//! What is asserted, and against what:
//!
//! * **passthrough** — an ordinary slot, over exactly the window's range;
//! * **read-native** — a slot with the kernel's read-only flag, over exactly the *rounded*
//!   write-trap span, with ordinary slots either side;
//! * **observe-everything** — the **absence** of a slot, which is a thing only the slot map
//!   can see. Nothing in the adapter's own bookkeeping distinguishes "no slot" from "a slot
//!   nobody looked at", which is why every assertion here reads the mock kernel's table.
//!
//! ★★ And the tiering is tied to [`Vmm::set_trap`], because a trap registration is a *claim*
//! about what a guest access does, and what it does is decided by the slot underneath it.

mod common;

use common::{
    BAR0_BASE, machine, overlay_gpa, overlay_len, overlay_trap, page, window_gpa, window_len,
};

use kayfabe_vmm::{BarId, HostRegion, TrapMode, Vmm, VmmError};
use kayfabe_vmm_qemu::mock_host::MockSlotRecord;
use kayfabe_vmm_qemu::slots::Tier;
use kayfabe_vmm_qemu::{TRAP_OVER_A_LIVE_SLOT, WRITE_TRAP_WITHOUT_A_READ_ONLY_SLOT, WindowSpec};

/// The port's "fill it from nothing" sentinel.
const FILL_FROM_NOTHING: HostRegion = HostRegion {
    id: u64::MAX,
    offset: 0,
};

/// ★★ **Passthrough is one slot over the whole window, and it is NOT read-only.**
///
/// The polarity is asserted explicitly. A device that set the read-only flag on every slot
/// would pass every *read* test in this suite and would make every guest store exit — a
/// correctness disaster that reads, from above, as "the window works".
#[test]
fn a_passthrough_window_is_exactly_one_read_write_slot_over_its_whole_range() {
    let (m, _host, slots) = machine();
    assert_eq!(
        slots.live(),
        vec![MockSlotRecord {
            slot: common::MOCK_CEILING - 1,
            gpa: window_gpa(),
            len: window_len(),
            readonly: false,
        }],
        "one slot, the whole window, read-WRITE"
    );
    assert_eq!(slots.read_only_installs(), 0);
    assert_eq!(
        m.tiers(),
        vec![(window_gpa(), window_len(), Tier::Passthrough)]
    );
    assert_eq!(m.audit().memslot_installs, 1);
    assert_eq!(m.audit().live_memslots, 1);
}

/// ★★★ **Read-native is a read-only slot over the rounded span, with passthrough either
/// side** — and the three pieces tile the window with no gap.
///
/// The rounding is the sharp part: read-native versus trapped is a **page** attribute in the
/// kernel's tables, so a sub-page request traps a whole page. A caller that assumed byte
/// granularity gets more trapped than it asked for — correct, and quietly slower — and the
/// assertion is on the *rounded* span because that is what physically became read-only.
#[test]
fn a_read_native_window_installs_a_read_only_slot_over_the_rounded_span_only() {
    let (m, _host, slots) = machine();
    let p = page();
    let mut v = m.vmm();

    // A sub-page write trap, deliberately: one byte.
    v.map_read_native(
        overlay_gpa(),
        overlay_len(),
        FILL_FROM_NOTHING,
        Some(overlay_gpa()..overlay_gpa() + 1),
    )
    .expect("a read-native window");

    let live = slots.live();
    // Only the slots of the read-native window; the realize-time BAR1 reservation is still
    // live and is not what this test is about.
    let ours: Vec<&MockSlotRecord> = live
        .iter()
        .filter(|r| r.gpa >= overlay_gpa() && r.gpa < overlay_gpa() + overlay_len())
        .collect();
    assert_eq!(
        ours.len(),
        2,
        "a two-page window with its FIRST page read-native is two slots: the read-only \
         page and the page after it. Got {ours:?}"
    );
    assert_eq!(
        (ours[0].gpa, ours[0].len, ours[0].readonly),
        (overlay_gpa(), p, true),
        "the first page must be READ-ONLY over exactly one page — the one-byte request \
         rounded out"
    );
    assert_eq!(
        (ours[1].gpa, ours[1].len, ours[1].readonly),
        (overlay_gpa() + p, p, false),
        "and the rest of the window must stay read-write; a read-only tail would make every \
         store into it exit for no reason"
    );
    assert_eq!(
        slots.read_only_installs(),
        1,
        "exactly one, not zero and not two"
    );

    assert_eq!(
        m.tiers()
            .into_iter()
            .filter(|(g, _, _)| *g >= overlay_gpa() && *g < overlay_gpa() + overlay_len())
            .collect::<Vec<_>>(),
        vec![
            (overlay_gpa(), p, Tier::ReadNative),
            (overlay_gpa() + p, p, Tier::Passthrough),
        ]
    );
}

/// ★★★ **Observe-everything is the ABSENCE of a slot**, and the pieces around it are still
/// installed.
///
/// This is the tier with nothing to point at: there is no object created, no flag set and no
/// call made. The only way to assert it is to look at what the kernel was told and find a
/// hole of exactly the right shape — which is why the mock kernel's table is public.
#[test]
fn an_observe_span_has_no_slot_at_all_and_leaves_a_hole_of_exactly_its_own_shape() {
    let p = page();
    let (m, _host, slots) = common::machine_with(
        kayfabe_vmm_qemu::mock_host::MockPolicy::default(),
        kayfabe_vmm_qemu::MachineConfig {
            windows: vec![WindowSpec {
                gpa: window_gpa(),
                len: 8 * p,
                // two disjoint observe holes, deliberately not at the edges
                observe: vec![
                    window_gpa() + 2 * p..window_gpa() + 3 * p,
                    window_gpa() + 5 * p..window_gpa() + 6 * p,
                ],
            }],
            ..common::config()
        },
    );

    assert_eq!(
        slots
            .live()
            .into_iter()
            .map(|r| (r.gpa - window_gpa(), r.len, r.readonly))
            .collect::<Vec<_>>(),
        vec![
            (0, 2 * p, false),
            (3 * p, 2 * p, false),
            (6 * p, 2 * p, false),
        ],
        "three slots with two holes; the holes are where every guest access must exit"
    );
    assert_eq!(m.audit().memslot_installs, 3);
    assert_eq!(
        m.tiers()
            .into_iter()
            .map(|(g, l, t)| (g - window_gpa(), l, t))
            .collect::<Vec<_>>(),
        vec![
            (0, 2 * p, Tier::Passthrough),
            (2 * p, p, Tier::Observe),
            (3 * p, 2 * p, Tier::Passthrough),
            (5 * p, p, Tier::Observe),
            (6 * p, 2 * p, Tier::Passthrough),
        ]
    );

    // ★ And the hole is still OUR memory from above: `gpa_read`/`gpa_write` serve the whole
    // window regardless of tier, because the tier decides what the GUEST's access does, not
    // what ours does. A device that refused its own read into an observe span could never
    // answer the trap it just took.
    let mut v = m.vmm();
    v.gpa_write(window_gpa() + 2 * p, &[0x5A; 16])
        .expect("our own write into an observe span");
    let mut back = [0u8; 16];
    v.gpa_read(window_gpa() + 2 * p, &mut back)
        .expect("and our own read");
    assert_eq!(back, [0x5A; 16]);
}

/// ★★★ **A trap registration is checked against the physical tier**, in both directions.
///
/// A read-write trap over a range a slot serves never fires: the guest's access resolves
/// from the slot and never leaves the guest. A write-only trap over a range with no
/// read-only slot never fires either. Both read as protection and are none — the exact
/// shape `testing_doctrine.md` calls a green instrument on an unexercised path, except that
/// here the *device* is the instrument.
#[test]
fn a_trap_is_refused_when_the_tier_underneath_it_cannot_make_it_fire() {
    let p = page();
    let (m, _host, _slots) = common::machine_with(
        kayfabe_vmm_qemu::mock_host::MockPolicy::default(),
        kayfabe_vmm_qemu::MachineConfig {
            // BAR0 0..16p is the read-write trap table row; put a passthrough window over
            // part of it so a read-write trap there is provably pointless.
            windows: vec![WindowSpec::passthrough(BAR0_BASE, 4 * p)],
            ..common::config()
        },
    );
    let mut v = m.vmm();

    assert_eq!(
        v.set_trap(BarId::Bar0, 0..2 * p, TrapMode::ReadWrite),
        Err(VmmError::Unsupported(TRAP_OVER_A_LIVE_SLOT)),
        "a read-write trap over a passthrough slot cannot fire"
    );
    // The same row, past the window: no slot there, so the trap is real.
    v.set_trap(BarId::Bar0, 8 * p..10 * p, TrapMode::ReadWrite)
        .expect("a read-write trap over an unbacked range is exactly what a trap is");

    // The write-only row (16p..18p) has no read-native tier under it yet.
    assert_eq!(
        v.set_trap(BarId::Bar0, 16 * p..17 * p, TrapMode::WriteOnly),
        Err(VmmError::Unsupported(WRITE_TRAP_WITHOUT_A_READ_ONLY_SLOT)),
        "a write-only trap with no read-only slot beneath it never sees a store"
    );
    v.map_read_native(
        overlay_gpa(),
        overlay_len(),
        FILL_FROM_NOTHING,
        Some(overlay_trap()),
    )
    .expect("the read-native window the row is about");
    v.set_trap(BarId::Bar0, 16 * p..17 * p, TrapMode::WriteOnly)
        .expect("and now the same registration is honest");
}

/// ★★ Every tier's slots are **cleared** when the window goes, and the conservation ledger
/// balances. A slot left live in the kernel over memory we unmapped is the one failure that
/// no amount of bookkeeping above it can detect.
#[test]
fn tearing_down_a_tiered_window_clears_every_slot_it_installed() {
    let p = page();
    let (m, _host, slots) = common::machine_with(
        kayfabe_vmm_qemu::mock_host::MockPolicy::default(),
        kayfabe_vmm_qemu::MachineConfig {
            windows: vec![WindowSpec {
                gpa: window_gpa(),
                len: 8 * p,
                observe: core::iter::once(window_gpa() + 4 * p..window_gpa() + 5 * p)
                    .collect::<Vec<_>>(),
            }],
            ..common::config()
        },
    );
    let mut v = m.vmm();
    v.map_read_native(
        overlay_gpa(),
        overlay_len(),
        FILL_FROM_NOTHING,
        Some(overlay_trap()),
    )
    .expect("a second, read-native window");

    let installed = slots.installs();
    assert_eq!(
        installed, 4,
        "2 passthrough + 1 read-only + 1 passthrough tail"
    );
    assert_eq!(slots.live().len(), 4);

    m.unrealize();

    assert!(
        slots.live().is_empty(),
        "unrealize must leave the kernel holding NOTHING; {:?} is a live mapping over \
         memory that has been unmapped",
        slots.live()
    );
    assert_eq!(
        slots.clears(),
        installed,
        "every install must be matched by a clear"
    );
    assert_eq!(m.audit().live_memslots, 0);
    assert_eq!(
        slots.replaces(),
        0,
        "and no install may ever have silently replaced a live slot"
    );
}

/// ★ A tier sub-range that leaves its window is refused before anything is installed.
#[test]
fn a_tier_outside_its_window_is_refused_and_installs_nothing() {
    let p = page();
    let (m, _host, slots) = machine();
    let before = slots.installs();
    // The window under test sits immediately after the realize-time one and is 4 pages
    // long, so `base .. base + 4p` is its whole extent. Each case below leaves it in a
    // different direction: past the end, straddling the end, empty, and before the start.
    let base = window_gpa() + window_len();
    for observe in [
        core::iter::once(base + 4 * p..base + 5 * p).collect::<Vec<_>>(),
        core::iter::once(base + 3 * p..base + 5 * p).collect::<Vec<_>>(),
        core::iter::once(base + p..base + p).collect::<Vec<_>>(),
        core::iter::once(base - p..base).collect::<Vec<_>>(),
    ] {
        let spec = WindowSpec {
            gpa: base,
            len: 4 * p,
            observe: observe.clone(),
        };
        assert!(
            m.install_tiered_window(&spec).is_err(),
            "{observe:?} is not inside the window it tiers"
        );
    }
    assert_eq!(
        slots.installs(),
        before,
        "a refused tier list must not have installed a slot on its way out"
    );
}
