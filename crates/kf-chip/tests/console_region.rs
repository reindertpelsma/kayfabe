// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ The boot display's console region (`docs/design/V3_DISPLAY.md` §4.11.4 row 1):
//! `fb_layout_with_console(fb_length, C)` is today's table, byte for byte, when the guest preserves
//! no console (`C = 0`), and a three-region table whose region 0 is exactly the console otherwise.

use kf_chip::bar0::{
    CONSOLE_ALIGN, ConsoleRefused, FW_CARVE_OUT_BYTES, fb_layout, fb_layout_with_console,
};

/// Store sizes the bench and the family lanes use (MiB), plus the smallest that holds the carve-out.
const SIZES_MB: [u64; 5] = [12288, 11904, 8192, 6144, 512];

/// 1920x1080 at a 7680-byte pitch, rounded up to 64 KiB: `kf_oprom::Geometry::for_mode(1920, 1080)`.
const C_1080P: u64 = 0x7F_0000;

#[test]
fn no_console_is_todays_layout_byte_for_byte() {
    for mb in SIZES_MB {
        let fb = mb << 20;
        assert_eq!(
            fb_layout_with_console(fb, 0),
            Ok(fb_layout(fb).expect("the store holds the carve-out")),
            "{mb} MiB"
        );
    }
}

#[test]
fn a_console_becomes_region_zero_and_nothing_else_moves() {
    for mb in SIZES_MB {
        let fb = mb << 20;
        let today = fb_layout(fb).unwrap();
        let l = fb_layout_with_console(fb, C_1080P).unwrap();
        assert_eq!(l.regions.len(), 3, "{mb} MiB: console, heap, carve-out");
        let (console, heap, carve) = (l.regions[0], l.regions[1], l.regions[2]);
        // region 0: exactly [0, C), reserved, scanned out (ISO), never compressed
        assert_eq!((console.base, console.limit), (0, C_1080P - 1));
        assert_eq!(
            console.reserved, C_1080P,
            "non-zero reserved => bRsvdRegion"
        );
        assert!(console.support_iso && !console.support_compressed && !console.protected);
        // region 1: today's heap, starting at C
        assert_eq!(heap.base, C_1080P);
        assert_eq!(heap.limit, today.regions[0].limit);
        assert_eq!(
            (
                heap.reserved,
                heap.performance,
                heap.support_compressed,
                heap.support_iso
            ),
            (0, 6, true, true)
        );
        // region 2 and both roots: unchanged
        assert_eq!(carve, today.regions[1]);
        assert_eq!(carve.reserved, FW_CARVE_OUT_BYTES);
        assert_eq!(
            (l.bar1_pde_base, l.bar2_pde_base, l.fb_length),
            (today.bar1_pde_base, today.bar2_pde_base, fb)
        );
        // contiguous, and the last limit is the store's last byte
        for w in l.regions.windows(2) {
            assert_eq!(w[0].limit + 1, w[1].base, "{mb} MiB: contiguous");
        }
        assert_eq!(l.regions[2].limit, fb - 1);
    }
}

#[test]
fn a_console_the_layout_cannot_hold_is_refused_by_name() {
    let fb = 8192u64 << 20;
    let carve = fb - FW_CARVE_OUT_BYTES;
    assert_eq!(
        fb_layout_with_console(fb, C_1080P + 0x1000),
        Err(ConsoleRefused::Unaligned {
            console: C_1080P + 0x1000
        })
    );
    assert_eq!(
        fb_layout_with_console(fb, carve),
        Err(ConsoleRefused::NotBelowCarveOut {
            console: carve,
            carve
        })
    );
    assert!(fb_layout_with_console(fb, carve - CONSOLE_ALIGN).is_ok());
    assert_eq!(
        fb_layout_with_console(0x1000_0000, 0),
        Err(ConsoleRefused::NoLayout {
            fb_length: 0x1000_0000
        })
    );
    // the names reach the log line
    let e = fb_layout_with_console(fb, 0x1234).unwrap_err().to_string();
    assert!(e.contains("0x1234") && e.contains("multiple"), "{e}");
}
