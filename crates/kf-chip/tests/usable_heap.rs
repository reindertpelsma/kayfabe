// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ P1+P2 / audit S1-43 (`docs/design/V3_P1P2_TSPACE.md` §7 test 17): a guest-named FB range — a
//! channel's USERD (its declared size, at least `NV_RAMUSERD_CHAN_SIZE`), a 16-byte error notifier —
//! is accepted only wholly inside ONE usable heap region of the layout kayfabe declared. Never the
//! firmware carve-out (where kayfabe's own BAR1/BAR2 roots live), never a preserved console, never
//! past the store.

use kf_abi::submit::USERD_SIZE;
use kf_chip::bar0::{FW_CARVE_OUT_BYTES, ROOT_PAGE_BYTES, fb_layout, fb_layout_with_console};

const FB: u64 = 12 << 30;
const C_1080P: u64 = 0x7F_0000;

#[test]
fn userd_and_notifier_bounded_to_the_usable_heap() {
    for console in [0, C_1080P] {
        let l = fb_layout_with_console(FB, console).expect("layout");
        let carve = l.carve();
        assert_eq!(carve, FB - FW_CARVE_OUT_BYTES);
        for len in [USERD_SIZE, 16] {
            // Inside the heap: its first and its last byte.
            assert!(l.in_usable_heap(console, len), "{console:#x} heap start");
            assert!(l.in_usable_heap(carve - len, len), "{console:#x} heap end");
            // Inside the carve-out, across its edge, at either declared root page, at the
            // store's last bytes, past the store, at 2^40.
            for (what, off) in [
                ("carve-out base", carve),
                ("across the carve-out edge", carve - len / 2),
                ("BAR1 root page", l.bar1_pde_base),
                ("BAR2 root page", l.bar2_pde_base),
                (
                    "BAR2 root page end",
                    l.bar2_pde_base + ROOT_PAGE_BYTES - len,
                ),
                ("store end", FB - 8),
                ("past the store", FB),
                ("2^40", 1 << 40),
                ("overflow", u64::MAX - 4),
            ] {
                assert!(
                    !l.in_usable_heap(off, len),
                    "console {console:#x}: {what} {off:#x}+{len:#x} accepted"
                );
            }
            // A zero-length range is not "inside".
            assert!(!l.in_usable_heap(console, 0));
        }
        // The roots really are in the carve-out (what the bound protects).
        assert!(l.bar1_pde_base >= carve && l.bar2_pde_base >= carve);
    }
    // Inside a preserved console: reserved, so refused.
    let l = fb_layout_with_console(FB, C_1080P).unwrap();
    assert!(!l.in_usable_heap(0, USERD_SIZE));
    assert!(
        !l.in_usable_heap(C_1080P - 8, 16),
        "straddles console and heap"
    );
    // Without a console the same offset is heap.
    assert!(fb_layout(FB).unwrap().in_usable_heap(0, USERD_SIZE));
}
