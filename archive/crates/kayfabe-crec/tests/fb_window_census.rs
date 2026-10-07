//! ★★★ **`#102` stage C — the guest writes its framebuffer through windows this port does
//! not model, and this is the number.**
//!
//! # The claim, and why it is a test rather than a paragraph
//!
//! `kayfabe_device::FbWindow` classifies three apertures — `PRAMIN`, the framebuffer
//! aperture and the instance/`BAR2` window — as **device memory rather than registers**, so
//! that an access to one is refused by name instead of falling into the *unclaimed register*
//! bucket that answers a defaulted zero. That classification is only worth its weight if the
//! guest actually uses those windows, and a design doc asserting it is a claim nobody can
//! re-run.
//!
//! So this file asks the **oracle** instead. `cap1_coldboot_hermetic` is 359 062 records of
//! a stock 580.159.04 open driver cold-booting a real GA106 against the C emulator, and it
//! records every trapped base-address-register access. Bucketing them through the
//! **production classifier** — not a reimplementation of it — gives the census below.
//!
//! ★ It is a *hermetic* capture (`m2fwd=off m2exec=off m2romregs=off`), so every access here
//! is the guest's own and nothing is attributable to a host GPU.
//!
//! # ⚠ The one translation this file performs, stated because it is a real fact and not a
//! convenience
//!
//! The recorder tags the instance window `bar = 3`, because a 64-bit framebuffer aperture
//! eats two PCI slots and RM's `BAR2` is therefore PCI `BAR3` (`C:
//! src/qemu/nvkvm_gpu_emul.c:6610`). `kayfabe_device` speaks **RM's** logical index, where
//! the same window is `bus_bar::INST == 2` (`ogkm-580:
//! src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:4709-4720`). The map is
//! spelled out in [`rm_bar_of`] rather than hidden in a `match` arm, because getting it
//! wrong would silently move 177 856 accesses into the wrong bucket and the test would still
//! be green.
//!
//! # ⊘ What this census does NOT pin, measured rather than assumed
//!
//! Poisoning `PRAMIN_BASE` upward by one page leaves this test **green**: the capture only
//! ever touches `0x702000`–`0x715ffc`, so a 1 MiB window anchored anywhere from `0x700000`
//! to `0x702000` still covers every access it contains. This file therefore pins *which
//! window the classifier attributes an access to* and *how many* — it does **not** pin the
//! windows' extents. Those are pinned separately and deliberately, by
//! `crates/kayfabe-device/tests/chip_table.rs`, which reads one byte past a declared
//! window's end and requires it to be a register again.
//!
//! Stated because the alternative is a reader assuming this test covers more than it does.
//! Swapping the framebuffer aperture and the instance window *does* turn it red, with the
//! two counts exchanged — that is the misattribution it is really for.

use std::collections::BTreeMap;

use kayfabe_crec::format::CKind;
use kayfabe_crec::{cap1_path, load_cap1};
use kayfabe_device::FbWindow;

/// The recorder's PCI slot index → RM's logical base-address-register index. See the module
/// docs. `None` is a slot RM does not enumerate.
fn rm_bar_of(rec_bar: u8) -> Option<u8> {
    match rec_bar {
        0 => Some(0), // registers
        1 => Some(1), // framebuffer aperture
        3 => Some(2), // instance / BAR2 window
        _ => None,
    }
}

#[test]
fn the_guest_writes_device_memory_through_windows_this_port_does_not_model() {
    let trace = match load_cap1() {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => panic!("cap1 at {:?} did not decode: {e:?}", cap1_path()),
        Err(e) => panic!("cap1 is missing at {:?} ({e})", cap1_path()),
    };
    let chip = &kayfabe_device::ga10x::GA106;

    let mut writes: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut reads: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut register_writes = 0u64;
    for r in trace.records() {
        let (is_write, bucket) = match r.kind {
            CKind::MmioWrite => (true, &mut writes),
            CKind::MmioRead => (false, &mut reads),
            _ => continue,
        };
        let Some(bar) = rm_bar_of(r.bar) else {
            continue;
        };
        // ★★ THE PRODUCTION CLASSIFIER. A local reimplementation here would make this test
        // agree with itself rather than with the code the guest meets.
        match chip.fb_window(bar, r.a) {
            Some(w) => *bucket.entry(w.name()).or_default() += 1,
            None => {
                if is_write {
                    register_writes += 1;
                }
            }
        }
    }

    // ★★★ THE CENSUS. Every number was produced by this harness against the committed
    // capture before it was written down (2026-07-31), and each is exact: an off-by-one
    // here means the classifier's extents moved.
    assert_eq!(
        writes.get(FbWindow::InstanceWindow.name()).copied(),
        Some(177_856),
        "the instance/BAR2 window carries the bulk of the guest's framebuffer traffic"
    );
    assert_eq!(
        writes.get(FbWindow::Pramin.name()).copied(),
        Some(33_978),
        "PRAMIN — the untranslated window, and the one that bootstraps BAR2's page tables"
    );
    assert_eq!(
        writes.get(FbWindow::FbAperture.name()).copied(),
        Some(2),
        "BAR1 is barely used during a cold boot — but it is not zero, and a cold boot is \
         the LEAST it is used (the matmul capture has 1511)"
    );
    assert_eq!(
        reads.get(FbWindow::InstanceWindow.name()).copied(),
        Some(541)
    );
    assert_eq!(reads.get(FbWindow::Pramin.name()).copied(), Some(11));

    // ★★ THE COMPARISON THAT MAKES THE NUMBER MEAN SOMETHING. Framebuffer writes outnumber
    // register writes by nearly sixty to one, so a plane that files them under "an unknown
    // register offset" is not mis-labelling a rounding error — it is mis-labelling almost
    // everything the guest writes.
    let fb_writes: u64 = writes.values().sum();
    assert_eq!(register_writes, 3_660);
    assert!(
        fb_writes > register_writes * 50,
        "framebuffer writes {fb_writes} vs register writes {register_writes}"
    );
}
