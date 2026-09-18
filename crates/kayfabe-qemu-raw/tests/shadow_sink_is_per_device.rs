//! ★★★★★ **w761b — the read-shadow sink may not outlive the device whose memory it names.**
//!
//! `[found by the safe→unsafe boundary audit, 2026-09-18]` `kayfabe_shim_bar0_shadow_attach`
//! stored `(base: *mut u8, len)` in a **process-lifetime** `static OnceLock`, whose segment
//! list was **push-only** — no removal anywhere, and `nvkvm_exit` has no detach. So:
//!
//! 1. device 1 attaches segments pointing into its own `MemoryRegion`s;
//! 2. `device_del` frees them;
//! 3. device 2 attaches its segments — pushed BEHIND device 1's;
//! 4. `ShadowSink::write` uses `.find()`, which returns the **first** match;
//! 5. a guest MMIO write then writes through device 1's freed pointer.
//!
//! A guest-reachable use-after-free, reached from **safe** code. ⊘ The `SAFETY` comment on
//! `ShadowSegment::base` asserted the premise that failed — *"owned by the device, which
//! outlives the register plane"* — true within one device lifetime, false across a recycle,
//! which `tests/device_recycle.rs` exists because this tree supports.
//!
//! These are source-level gates: the property is a LIFETIME, and a runtime test would need two
//! real QEMU devices and a freed region to observe it — i.e. it could only fail by corrupting
//! memory. Pinning the ownership is the check that can fail safely.

const SHIM: &str = include_str!("../src/shim.rs");
const SHIM_UNSAFE: &str = include_str!("../src/shim_unsafe.rs");

/// The sink is owned by `Regs`, which `kayfabe_shim_regs_destroy` drops.
#[test]
fn the_sink_hangs_off_regs_and_not_a_static() {
    assert!(
        SHIM.contains("read_shadow: std::sync::OnceLock<Arc<crate::shim_unsafe::ShadowSink>>"),
        "★★★★★ `Regs` no longer owns the read-shadow sink. If it moved back to a `static`, a \
         segment outlives the device whose memory it points at, and a guest MMIO write writes \
         through freed memory."
    );
    // ⊘ Match the DECLARATION (`static NAME:`), never the bare name: this file's own
    // explanatory comment says `static SHADOW_SINK` and tripped the first draft of this gate.
    // Same class as the I1 loop gate that a comment containing the word `while` broke earlier
    // today — a gate that greps raw source must exclude prose or it fails on its own docs.
    assert!(
        !SHIM_UNSAFE.contains("static SHADOW_SINK:"),
        "★★★★★ the process-lifetime `static SHADOW_SINK` is back. Its segment list is \
         push-only, so every device recycle leaves a stale `*mut u8` that `.find()` returns \
         BEFORE the live one."
    );
}

/// ⊘ The bound must be overflow-safe on BOTH sums: `off` is the guest's own MMIO offset.
#[test]
fn the_segment_bound_uses_checked_add_on_both_sums() {
    let at = SHIM_UNSAFE
        .find("impl kayfabe_device::plane::ReadShadowPort for ShadowSink")
        .expect("the sink still implements the port");
    let body = &SHIM_UNSAFE[at..at + 2000];
    assert!(
        body.contains("off.checked_add(") && body.contains("s.off.checked_add("),
        "★★★ the segment search is back to plain `+`. A wrap on either sum makes the \
         containment test pass for a span that is NOT inside the segment, and the write that \
         follows is out of bounds — the exemplar is `kayfabe-linux-raw/src/bounds.rs:78`."
    );
}
