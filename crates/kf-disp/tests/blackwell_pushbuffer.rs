// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-11-03 (Blackwell Windows display, `traces/blackwell_win_20261103/`): a display channel's DMA
//! pushbuffer format is the CLASS's, not Volta's. `NVC37D`..`NVC77D` (and their window classes) have
//! `DMA_METHOD_OFFSET 13:2` and `DMA_JUMP_OFFSET 11:2` (a 4 KiB ring); `NVCA7D`/`NVCA7E`/`NVCA7B` (GB20x)
//! have `15:2` for both (a 64 KiB ring, methods up to `0xfffc`: `TILE_SET_*` at `0x6000`,
//! `WINDOW_SET_PHYSICAL_WINDOW` up to `0x8000`). The decoder was fixed at 13:2 / 11:2 / 4 KiB, so on
//! GB203 the Windows driver's core channel (64 KiB, `channelPBSize` 4) was halted at `PUT 0x1d70`
//! ("outside the 4096-byte pushbuffer", MEASURED `diagA_b1_qemu.log.gz`) and every modeset was lost.
//! Every width below comes from the generated class table.

use kf_disp::class::{ClassTable, for_version};
use kf_disp::pushbuf::{DecodeError, Format, MethodWrite, decode};

const VERSIONS: [&str; 5] = [
    "580.159.04",
    "580.65.06",
    "580.94.02",
    "590.48.01",
    "595.91.07",
];

fn table(v: &str) -> &'static ClassTable {
    for_version(v).unwrap()
}

fn ring(bytes: u32, words: &[(u32, u32)]) -> Vec<u8> {
    let mut b = vec![0u8; bytes as usize];
    for (at, w) in words {
        b[*at as usize..*at as usize + 4].copy_from_slice(&w.to_le_bytes());
    }
    b
}

/// An incrementing method header in `f`'s layout: `count` words at `method`.
fn header(f: &Format, count: u32, method: u32) -> u32 {
    (count << 18) | (((method >> 2) & ((1 << (f.method.0 - 1)) - 1)) << 2)
}

#[test]
fn every_dma_class_resolves_its_own_format() {
    for v in VERSIONS {
        let t = table(v);
        for c in [0xC57D, 0xC67D, 0xC77D, 0xC57E, 0xC67E, 0xC57B, 0xC67B] {
            let f = Format::resolve(t, c).unwrap_or_else(|| panic!("{v} NV{c:04X}"));
            assert_eq!(
                (f.method, f.jump, f.bytes()),
                ((13, 2), (11, 2), 4096),
                "{v} NV{c:04X}"
            );
        }
        for c in [0xCA7D, 0xCA7E, 0xCA7B] {
            let f = Format::resolve(t, c).unwrap_or_else(|| panic!("{v} NV{c:04X}"));
            assert_eq!(
                (f.method, f.jump, f.bytes()),
                ((15, 2), (15, 2), 65536),
                "{v} NV{c:04X}"
            );
        }
    }
}

/// The GB203 wall measured on 2026-11-03 (RTX 5070 Ti): a core ring of 64 KiB whose PUT is past 4 KiB decodes; a method at
/// `0x6000` (`NVCA7D_TILE_SET_TILE_SIZE(0)`) is that method, not `0x2000`; a JUMP reaches `0x8000`.
#[test]
fn a_gb20x_core_ring_decodes_past_4k_with_its_own_offsets() {
    let t = table("580.65.06");
    let f = Format::resolve(t, 0xCA7D).unwrap();
    let tile = t.a(0xCA7D, "TILE_SET_TILE_SIZE", 0);
    let tile = tile.unwrap_or(0x6000);
    let jump = (1 << 29) | 0x8000;
    let b = ring(
        f.bytes(),
        &[
            (0x1d00, header(&f, 1, tile)),
            (0x1d04, 7),
            (0x1d08, jump),
            (0x8000, header(&f, 1, 0x200)),
            (0x8004, 9),
        ],
    );
    let (w, get, err) = decode(&f, &b, 0x1d00, 0x8008);
    assert_eq!(err, None);
    assert_eq!(get, 0x8008);
    assert_eq!(
        w,
        vec![
            MethodWrite {
                method: tile,
                data: 7
            },
            MethodWrite {
                method: 0x200,
                data: 9
            },
        ]
    );
}

/// The Ada (and earlier) format is unchanged: 4 KiB, and a ring larger than its class can address is
/// refused, never read.
#[test]
fn an_ada_ring_keeps_its_4k_bound() {
    let f = Format::resolve(table("580.65.06"), 0xC77D).unwrap();
    let b = ring(f.bytes(), &[(0, header(&f, 1, 0x200)), (4, 1)]);
    assert_eq!(decode(&f, &b, 0, 8).2, None);
    let big = ring(8192, &[(0, header(&f, 1, 0x200)), (4, 1)]);
    assert_eq!(decode(&f, &big, 0, 8).2, Some(DecodeError::BadPointers));
}
