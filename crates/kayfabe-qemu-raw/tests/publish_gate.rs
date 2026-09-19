//! ★★★★★ **THE PUBLICATION DIRTY GATE — both terms, and why one of them is not ours.**
//!
//! `Regs::publish_vas_rows` skips a VAS whose census it has already taken, replaying the old
//! line. A wrong skip means declared rows never reach the host VA space, and the GPU then
//! faults on an address the guest believes it mapped.
//!
//! # ⊘ The gate was NOT the w784 bug — and that is exactly why it needs a test
//!
//! `[measured w784]` every publication pass of an 831-window boot replayed its last census
//! (`⊘SKIPPED(w318 dirty gate … unchanged since the last COMPLETED pass)`), and the cause was
//! two layers away in `Vas::publish_epoch`, whose guest-side term had gone dark under the
//! single store. **The gate did its job correctly on a stuck input.**
//!
//! ⚠ But there was nothing to call and nothing to unit-test, so telling those two apart —
//! *"the gate is over-skipping"* versus *"the epoch is not moving"* — cost a rented GPU and a
//! guest boot. With the contract pinned here, the next person who suspects this gate can be
//! answered in ten milliseconds and sent one layer down.
//!
//! ⊘ These tests deliberately assert the CONTRACT (both terms, independently) and never how
//! the caller obtains either value. A test that also built an epoch would be testing
//! `publish_epoch`, which `kayfabe-tests/tests/publish_epoch_gate.rs` already does.

use kayfabe_qemu_raw::shim::publish_gate_is_clean;

const E0: (u64, usize) = (0xabc_def, 3);
const E1: (u64, usize) = (0xabc_df0, 3);

/// ⊘ **The skipping case** — and the gate must still have one, or w763z's cost returns.
#[test]
fn identical_epoch_and_joined_is_clean() {
    assert!(
        publish_gate_is_clean(E0, 7, E0, 7),
        "⊘ nothing about this VAS or the host's joined ranges has changed since the census \
         that produced the cached line. Re-walking is pure cost"
    );
}

/// ★★★ **OUR TERM.** A moved epoch must re-arm the pass even when host state is identical.
#[test]
fn a_moved_epoch_is_dirty_even_when_the_host_is_unchanged() {
    assert!(
        !publish_gate_is_clean(E0, 7, E1, 7),
        "★ the guest changed its address space. Skipping here is `published=0` on a VAS with \
         rows waiting, which is `FAULT_PDE` one doorbell later"
    );
}

/// ★★★ **THE HOST TERM, and it is the one a reader forgets.** The refusals this gate skips
/// are *"that framebuffer range is already joined"* — an outcome that depends on host state,
/// not on our table. A join or a release must re-arm even when our rows are untouched.
#[test]
fn a_changed_joined_count_is_dirty_even_when_our_epoch_is_unchanged() {
    assert!(
        !publish_gate_is_clean(E0, 7, E0, 8),
        "★ a framebuffer range was joined since the cached census. The rows it would have \
         rescued were counted as refused in that line, and replaying it hides them"
    );
    assert!(
        !publish_gate_is_clean(E0, 7, E0, 6),
        "★ and a RELEASE moves it the other way — the count is not monotonic, so a gate that \
         only noticed growth would skip after every teardown"
    );
}

/// ⊘ **Neither term can mask the other.** If both moved, the answer is still dirty — the
/// assertion that rules out a gate keyed on one term that merely happens to track the other.
#[test]
fn both_terms_moving_is_still_dirty() {
    assert!(!publish_gate_is_clean(E0, 7, E1, 8));
}

/// ★★ **The epoch is a PAIR and both halves count.** Its second element is the guest-RAM pin
/// count; a gate comparing only the hash would skip a VAS that gained a pin.
#[test]
fn the_pin_half_of_the_epoch_is_not_ignored() {
    let same_hash_more_pins = (E0.0, E0.1 + 1);
    assert!(
        !publish_gate_is_clean(E0, 7, same_hash_more_pins, 7),
        "★ `publish_epoch` returns `(hash, guest_ram_pins.len())` and the census buckets \
         `already_pinned` from that map. A gate reading only the hash replays a census whose \
         pin bucket is now wrong"
    );
}
