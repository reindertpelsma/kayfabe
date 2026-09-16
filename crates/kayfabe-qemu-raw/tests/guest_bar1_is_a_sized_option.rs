//! ★★★★★ **w734 — §w727's BAR1 KNOB, AND THE THREE REFUSALS IT MUST KEEP APART.**
//!
//! `[surveyed w732]` `Bar1Choice::check` had **no caller anywhere in the tree outside its own
//! tests**: §w727's *"refuse, never clamp"* was fully implemented and **unreachable**, while
//! GA106's BAR1 stayed the hardcoded `const FB_WINDOW_LEN = 256 << 20`. w734 wires it. This is
//! the offline known-positive for the wiring — no GPU, no board, no boot.
//!
//! ⊘ What it does NOT test: that a boot with the knob set actually advertises the new
//! aperture to a guest. That needs the hypervisor, and the hypervisor has its own refusal for
//! the one way it can go wrong (`nvkvm.c:3552-3559`, `bar1-size` vs the chip row). Said here
//! so the coverage is not overread.

use kayfabe_qemu_raw::bar1budget::{
    BAR1_MIN_BYTES, Bar1Choice, HostBar1, OUR_HEADROOM_BYTES, guest_bar1_from,
};

/// ★ Unset is the default and is **not** an error — §w727 item 3's other half: *"do not gate
/// on it before anything consumes BAR1 views; a refusal that fires every boot for a design not
/// yet switched on is noise that teaches people to ignore the check."*
#[test]
fn unset_is_the_chip_rows_own_aperture_and_is_not_an_error() {
    assert_eq!(guest_bar1_from(None).expect("unset parses"), None);
    assert_eq!(guest_bar1_from(Some("  ")).expect("blank parses"), None);
}

/// ★★★ **A TYPO IS REFUSED, NOT DEFAULTED.** Both directions of a silent default are bad and
/// they are bad in opposite ways; this is the one that runs the control arm on a boot the
/// operator believes is armed.
#[test]
fn a_value_that_is_not_a_number_is_refused_rather_than_ignored() {
    let e = guest_bar1_from(Some("128M")).expect_err("`128M` is not a whole number of MiB");
    assert!(e.contains("not a whole number"), "{e}");
    assert!(
        e.contains("control arm"),
        "the refusal must say WHY silence would be worse, or the next person deletes it: {e}"
    );
}

/// ★★★ **POWER OF TWO** — §w727 item 1. *"A 'select any size' knob that accepts 100 MiB is a
/// bug the guest's enumeration finds, not us."*
#[test]
fn a_non_power_of_two_is_refused_by_that_name() {
    let e = guest_bar1_from(Some("100")).expect_err("100 MiB is not a power of two");
    assert!(e.contains("power of two"), "{e}");
    // The three refusals must be distinguishable: each names a different fix.
    assert!(!e.contains("below the"), "{e}");
}

/// ★★★ **THE MINIMUM** — §w727 item 2, and the refusal must carry that it is PROVISIONAL.
/// `[measured]` the one workload's BAR1 working set is 912 pages ≈ 3.6 MiB; 64 MiB clears it
/// by ~18×, *"but that is ONE workload"*.
#[test]
fn below_the_minimum_is_refused_and_says_the_minimum_is_provisional() {
    let e = guest_bar1_from(Some("32")).expect_err("32 MiB is below the floor");
    assert!(e.contains("below the"), "{e}");
    assert!(
        e.contains("PROVISIONAL"),
        "a floor whose provenance is one workload must say so where it refuses, or it \
         hardens into a spec: {e}"
    );
    assert_eq!(BAR1_MIN_BYTES, 64 * 1024 * 1024);
}

/// ★★★★★ **THE FIT CHECK IS THE ONE THAT WAS UNREACHABLE** — and it must refuse the number
/// this tree actually ships. `[measured w726/e36]` on the bench GA106: `256 + 16 > 256`.
#[test]
fn the_shipped_256_does_not_fit_a_256_mib_board_and_128_does() {
    let host = HostBar1::Bytes(256 * 1024 * 1024);
    let asked = Bar1Choice::parse(256).expect("256 is a legal size");
    let e = asked
        .check(host)
        .expect_err("256 + our headroom cannot fit a 256 MiB aperture");
    let txt = e.to_string();
    assert!(txt.contains("does not fit"), "{txt}");
    assert!(
        txt.contains("128"),
        "the refusal must name the largest size that WOULD fit — a message that is a fix, \
         not a complaint: {txt}"
    );
    Bar1Choice::parse(128)
        .expect("128 is a legal size")
        .check(host)
        .expect("128 + 16 <= 256");
}

/// ⊘ **AN UNKNOWN BOARD IS NOT A REFUSAL.** *"Refusing a boot because we could not read sysfs
/// would be the instrument deciding the experiment."*
#[test]
fn an_unreadable_host_aperture_does_not_refuse_the_boot() {
    Bar1Choice::parse(256)
        .expect("legal size")
        .check(HostBar1::Unknown("no NVIDIA device"))
        .expect("an unknown board yields no verdict, and no verdict is not a refusal");
}

/// ★★★★★ **THE CHIP ROW ACTUALLY MOVES, AND THE `bool` REPORTS IT.**
///
/// ⊘ w614's failure in this tree was a patch that matched nothing and reported success. The
/// knob's patcher returns whether the row moved, and BOTH answers are checked here: a real
/// change moves it, and asking for what the row already says does not — because an operator
/// reading `row_moved=false` beside the size they asked for must be able to tell *"already
/// so"* from *"silently ignored"*.
#[test]
fn patching_the_bar_row_moves_it_and_says_when_it_did_not() {
    use kayfabe_abi::pcibars::bus_bar;
    let base = kayfabe_device::ga10x::ga106_profile(kayfabe_device::ga10x::FB_SIZE_MB);
    let before = base.pci_bar_len(bus_bar::FB);
    assert_eq!(before, 256 * 1024 * 1024, "the shipped row");

    let (patched, moved) = kayfabe_device::with_bar_len(base, bus_bar::FB, 128 * 1024 * 1024);
    assert!(moved, "128 MiB differs from 256 MiB, so the row must move");
    assert_eq!(patched.pci_bar_len(bus_bar::FB), 128 * 1024 * 1024);
    // ⊘ Everything else about the board is the same board.
    assert_eq!(patched.pci_device_id, base.pci_device_id);
    assert_eq!(
        patched.pci_bar_len(bus_bar::INST),
        base.pci_bar_len(bus_bar::INST),
        "sizing BAR1 must not move BAR2"
    );
    assert_eq!(
        patched.pci_bar_len(bus_bar::REGS),
        base.pci_bar_len(bus_bar::REGS),
        "sizing BAR1 must not move the register aperture"
    );

    let (same, moved) = kayfabe_device::with_bar_len(base, bus_bar::FB, before);
    assert!(!moved, "asking for what the row says is a no-op, reported");
    assert!(std::ptr::eq(same, base), "and it must not leak a copy");
}

/// ⚠ **A BAR THE CHIP DOES NOT PRESENT IS LEFT ALONE.** Row 3 of a GA106's table is the I/O
/// BAR and is `size_bytes = 0`, which is RM's own spelling for *"absent"*. Sizing it would
/// INVENT an aperture, and §22 has no defence against an aperture the board does not have.
#[test]
fn an_absent_bar_is_not_sized_into_existence() {
    use kayfabe_abi::pcibars::bus_bar;
    let base = kayfabe_device::ga10x::ga106_profile(kayfabe_device::ga10x::FB_SIZE_MB);
    assert_eq!(base.pci_bar_len(3), 0, "the I/O BAR is absent on a GA106");
    let (p, moved) = kayfabe_device::with_bar_len(base, 3, 64 * 1024 * 1024);
    assert!(!moved, "an absent BAR must not be sized into existence");
    assert_eq!(p.pci_bar_len(3), 0);
    // And an index past the table is the same answer, not a panic.
    let (p, moved) = kayfabe_device::with_bar_len(base, 99, 64 * 1024 * 1024);
    assert!(!moved);
    assert_eq!(p.pci_bar_len(bus_bar::FB), base.pci_bar_len(bus_bar::FB));
}

/// ⊘ The headroom is a number `bar1budget.rs` owns and can be wrong; pinned here so a change
/// to it is a change a reviewer sees, rather than a silently different verdict.
#[test]
fn the_headroom_is_a_stated_number_not_a_derived_one() {
    assert_eq!(OUR_HEADROOM_BYTES, 16 * 1024 * 1024);
}

/// ★★★★★ **w734 — THE FB-SIZE DERIVATION SILENTLY UNDOES THE BAR1 PATCH.**
///
/// `[measured w734, boot `w734bar1`]` This is the defect the first knob boot found, and a
/// green raw client (`(P)`, `THREADS 8 of 8`) did **not** catch it.
///
/// `ga106_profile` rebuilds from `let mut p = GA106;` — the **static** row — so it restores
/// `pci_bars = GA106_PCI_BARS`, whose framebuffer window is the hardcoded `256 << 20`. On that
/// boot `chip_identity` (which QEMU checks its own `bar1-size` against, and which does NOT go
/// through the derivation) answered 128 MiB, the hypervisor registered 128 MiB, and the plane
/// — hence the emulated GSP's `BUS_GET_PCI_BAR_INFO` — told the guest **256 MiB**.
///
/// ⚠ That is exactly what `nvkvm.c:3552-3559` exists to refuse, reached through a door that
/// check cannot see: the two numbers **it** compares were both 128.
///
/// ⊘ Pinned here rather than only fixed at the call site, because the property under test is
/// a fact about `ga106_profile` that the next reader will not expect.
#[test]
fn deriving_the_framebuffer_size_restores_the_static_bar_table() {
    use kayfabe_abi::pcibars::bus_bar;
    let base = kayfabe_device::ga10x::ga106_profile(kayfabe_device::ga10x::FB_SIZE_MB);
    let (patched, moved) = kayfabe_device::with_bar_len(base, bus_bar::FB, 128 * 1024 * 1024);
    assert!(moved);
    assert_eq!(patched.pci_bar_len(bus_bar::FB), 128 * 1024 * 1024);

    // The size derivation takes MiB, not a profile — so it cannot carry the patch forward.
    let derived = kayfabe_device::ga10x::ga106_profile(4096);
    assert_eq!(
        derived.pci_bar_len(bus_bar::FB),
        256 * 1024 * 1024,
        "★★★ the derivation rebuilds from the STATIC row and the BAR1 patch is GONE. The fix \
         is ordering — re-apply the knob AFTER every other patch — plus the agreement gate \
         that makes the class a named refusal instead of a one-time fix."
    );
    assert_eq!(
        derived.fb_length,
        4096 << 20,
        "and it did do its own job, which is why this was invisible"
    );

    // ⇒ The order that is correct: derive the size first, then size the BAR.
    let (both, moved) = kayfabe_device::with_bar_len(derived, bus_bar::FB, 128 * 1024 * 1024);
    assert!(moved);
    assert_eq!(both.pci_bar_len(bus_bar::FB), 128 * 1024 * 1024);
    assert_eq!(
        both.fb_length,
        4096 << 20,
        "and the size survives the BAR patch"
    );
}
