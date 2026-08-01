//! ★★★ **The BAR0 moving window (`#146`)** — the rung the 2026-08-01 `evt1` boot stopped
//! at, tested as the guest exercises it.
//!
//! # What the guest does, and why "it read back" is not the whole test
//!
//! `kbusVerifyBar2_GM107` (`ogkm-580: kern_bus_gm107.c:4084-4090`) programs
//! `NV_PBUS_BAR0_WINDOW`, then does a plain dword **write-then-read through `PRAMIN`**.
//! Making *that one access* work is easy and is not what this file is for.
//!
//! ⊘ The warning that came with the rung (`docs/design/boot_measured_2026_08_01.md` §18) is
//! that **`kbusInitBar2` programs the same window and never reads any of it back**, so a
//! window that silently drops writes lets every earlier step return `NV_OK` and is caught
//! only at the verify, hundreds of operations later, as `NV_ERR_MEMORY_ERROR`. So these
//! tests are written against the three ways a write can be lost rather than against the one
//! access that would notice:
//!
//! 1. the window register not being a **latch**, so the guest's own read-modify-write and
//!    RM's `cachedBar0WindowVidOffset` both silently mis-point it;
//! 2. the read path and the write path resolving an offset to **different addresses**;
//! 3. a store that answers a write with **success and no bytes**.
//!
//! ★ Each of the three has a test whose failure is a *statement about the mechanism*, not
//! about the one address the driver happens to use.

use kayfabe_device::fbwin::{
    Bar0Window, FB_PAGE, FbRefused, FbStore, OUTSIDE_FRAMEBUFFER, RESIDENT_CAP_REACHED, RefusingFb,
    SparseFb,
};
use kayfabe_device::plane::{ReadOutcome, RegPlane};
use kayfabe_device::{FbWindow, NanoClock, SteppingClock, abi};

/// `NV_PBUS_BAR0_WINDOW`, transcribed rather than read off the chip row — this file must be
/// the **second** description, the one that disagrees when the first one moves.
const WINDOW_REG: u64 = 0x0000_1700;
/// `DRF_BASE(NV_PRAMIN)` (`ogkm-580: dev_ram.h:26`), likewise transcribed.
const PRAMIN: u64 = 0x0070_0000;
/// `NV_PRAMIN`'s length — 1 MiB.
const PRAMIN_LEN: u64 = 0x0010_0000;
/// GA106's advertised framebuffer, 12 GiB.
const FB_LEN: u64 = 12288 << 20;

/// The address the 2026-08-01 boot measured RM handing `kbusVerifyBar2` — the top of
/// GA106's usable framebuffer region.
const BENCH_ADDR: u64 = 0x0002_EFBA_E000;
/// Its window base, `bar0TestAddr >> 16`.
const BENCH_BASE: u32 = 0x0002_EFBA;
/// Its window offset, `bar0TestAddr & 0xffff`.
const BENCH_OFF: u64 = 0xE000;
/// `SAMPLEDATA`, the constant `kbusVerifyBar2_GM107` writes.
const SAMPLEDATA: u64 = 0xABCD_ABCD;

fn plane() -> RegPlane {
    let p = RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    p.set_fb(Box::new(SparseFb::new(FB_LEN)));
    p
}

/// A plane with **no** framebuffer store, for the refusal half.
fn bare_plane() -> RegPlane {
    RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable")
}

/// `GPU_FLD_WR_DRF_NUM(pGpu, _PBUS, _BAR0_WINDOW, field, v)` — RM's own **read**-modify-
/// write, spelled out.
///
/// ★★★ This helper is the test, in a sense: writing the register with a whole word would
/// pass on a device that answers the register with a defaulted zero, and the guest does not
/// do that. It reads first.
fn fld_wr(p: &RegPlane, shift: u32, mask: u32, v: u32) {
    let cur = p.read(0, WINDOW_REG, 4).value() as u32;
    let next = (cur & !(mask << shift)) | ((v & mask) << shift);
    let _ = p.write(0, WINDOW_REG, 4, u64::from(next));
}

const BASE_SHIFT: u32 = 0;
const BASE_MASK: u32 = 0x00FF_FFFF;
const TARGET_SHIFT: u32 = 24;
const TARGET_MASK: u32 = 0x3;
/// `NV_PBUS_BAR0_WINDOW_TARGET_SYS_MEM_COHERENT` — deliberately not `_VID_MEM`'s zero, so
/// a device that dropped the field would look the same as one that kept it.
const TARGET_SYS_COHERENT: u32 = 2;

// =====================================================================================
// 1. The register is a LATCH — the failure mode with no other symptom
// =====================================================================================

/// ★★★ **The guest's own two-step field update composes.**
///
/// `kbusVerifyBar2_GM107` writes `_BASE` and then `_TARGET`, each through
/// `GPU_FLD_WR_DRF_NUM`, which reads the register first. A device answering the register
/// with a defaulted zero would have the **second** write put the base back to zero, and
/// every access afterwards would be mis-addressed with nothing logged anywhere.
#[test]
fn the_window_register_read_modify_write_composes_because_it_is_a_latch() {
    let p = plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, BENCH_BASE);
    fld_wr(&p, TARGET_SHIFT, TARGET_MASK, TARGET_SYS_COHERENT);

    let raw = p.read(0, WINDOW_REG, 4).value() as u32;
    assert_eq!(
        raw & BASE_MASK,
        BENCH_BASE,
        "★ the BASE the FIRST write set must survive the SECOND write; a defaulted-zero \
         read here zeroes it and nothing says so"
    );
    assert_eq!((raw >> TARGET_SHIFT) & TARGET_MASK, TARGET_SYS_COHERENT);
}

/// ★★★ **A bit this port does not decode survives the round trip.**
///
/// The register is read-modify-written by the guest, so every bit it reads back is a bit it
/// will write again. A device that masked the word down to the two fields it understands
/// would silently clear anything else on the guest's *next* field update — which is a
/// read-modify-**lose** at a register whose entire job is to be modified in place, and the
/// class of defect is exactly the one `#146` is about: no symptom here, a symptom much
/// later, attributed to something else.
///
/// ⊘ Bits 31:26 are reserved in `dev_bus.h:43-50`. This port models none of them and
/// therefore may not *decide* anything about them either.
#[test]
fn a_reserved_bit_this_port_does_not_decode_still_reads_back() {
    let p = plane();
    let odd = 0x8400_0000u32 | (u64::from(BENCH_BASE) as u32);
    let _ = p.write(0, WINDOW_REG, 4, u64::from(odd));
    assert_eq!(
        p.read(0, WINDOW_REG, 4).value() as u32,
        odd,
        "★ the whole word, verbatim — a mask here loses the guest's own bits on its next \
         read-modify-write"
    );
    // …and the address the window resolves is still taken from BASE alone.
    let w = p.write(0, PRAMIN + BENCH_OFF, 4, SAMPLEDATA);
    assert_eq!(w.fb_landed, Some(BENCH_ADDR));
}

/// ★★★ **RM's own cache is refreshed from this register**, so a dropped write is permanent.
///
/// `kbusSetBAR0WindowVidOffset_GM107` (`ogkm-580: kern_bus_gm107.c:4728-4760`) keeps
/// `cachedBar0WindowVidOffset`, seeds it from `GPU_REG_RD_DRF(_BAR0_WINDOW, _BASE)` while it
/// is zero, and then **skips the register write entirely** whenever the cache already holds
/// the offset asked for. A device that dropped the first write leaves RM believing the
/// window moved, with no later write to correct it.
///
/// This test is that function, transcribed, run twice.
#[test]
fn rms_own_window_cache_stays_true_because_the_register_answers_what_it_was_given() {
    let p = plane();
    let mut cached: u64 = 0;
    let set = |vid_offset: u64, cached: &mut u64| {
        if *cached == 0 {
            *cached = u64::from(p.read(0, WINDOW_REG, 4).value() as u32 & BASE_MASK) << 16;
        }
        if *cached != vid_offset {
            fld_wr(&p, BASE_SHIFT, BASE_MASK, (vid_offset >> 16) as u32);
            fld_wr(&p, TARGET_SHIFT, TARGET_MASK, 0);
            *cached = vid_offset;
        }
    };

    let want = u64::from(BENCH_BASE) << 16;
    set(want, &mut cached);
    // ★ The second call is the one that matters: RM skips the register write, so from here
    // on the ONLY thing keeping the device and RM in agreement is that the first write
    // landed.
    set(want, &mut cached);

    let _ = p.write(0, PRAMIN + BENCH_OFF, 4, SAMPLEDATA);
    let out = p.read(0, PRAMIN + BENCH_OFF, 4);
    assert_eq!(
        out,
        ReadOutcome::Fb {
            window: FbWindow::Pramin,
            phys: BENCH_ADDR,
            value: SAMPLEDATA
        },
        "the window must still be where RM's cache says it is"
    );
}

// =====================================================================================
// 2. ONE address function — the read and the write cannot disagree
// =====================================================================================

/// ★★★ **`kbusVerifyBar2_GM107:4084-4090`, at the exact address one boot printed.**
///
/// `[measured]` boot `evt1`, 2026-08-01, rev `0d82456`, stock 580.159.04 guest
/// (`docs/design/boot_measured_2026_08_01.md` §15): *"Address 0x2efbae000 programmed through
/// the bar0 window with value 0xabcdabcd did not read back the last write."* ⊘ One boot, one
/// guest, one allocation — the address is where RM's heap happened to place a 16-byte
/// allocation, not a constant, which is why the property test above sweeps bases instead of
/// resting on this one.
#[test]
fn the_verify_bar0_window_subtest_passes_at_the_address_the_boot_measured() {
    let p = plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, BENCH_BASE);
    fld_wr(&p, TARGET_SHIFT, TARGET_MASK, 0);

    // `testData = GPU_REG_RD32(...)` — the driver reads first, and fresh framebuffer is
    // zero rather than a refusal. See `fbwin`'s module docs for why that is a statement.
    assert_eq!(p.read(0, PRAMIN + BENCH_OFF, 4).value(), 0);

    let w = p.write(0, PRAMIN + BENCH_OFF, 4, SAMPLEDATA);
    assert_eq!(w.fb_landed, Some(BENCH_ADDR), "it must LAND, and say where");
    assert!(w.fb_refusal.is_none());
    assert!(
        w.fb_window.is_none(),
        "⊘ and it must NOT be reported as a dropped window write — that arm is for the \
         two GMMU-translated windows this port has no address model for"
    );

    assert_eq!(
        p.read(0, PRAMIN + BENCH_OFF, 4).value(),
        SAMPLEDATA,
        "★★★ …did not read back the last write — the guest's own error message"
    );
}

/// ★★★ **The full 24-bit base, and the arithmetic is `+` rather than `|`.**
///
/// `PRAMIN` is 1 MiB and the window origin is only 64 KiB-aligned, so a window offset above
/// `0xFFFF` overlaps the origin's low bits. With `|`, base `1` + offset `0x18000` would
/// resolve to `0x18000` — aliasing it onto the window at base `0`. With `+` it is
/// `0x28000`. The two are indistinguishable at every offset the *verify* uses, which is why
/// this is tested at an offset it does not.
#[test]
fn the_window_offset_is_added_to_the_origin_and_not_or_ed_into_it() {
    let p = plane();
    // Window at origin 0: write a marker at framebuffer 0x18000.
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0);
    let a = p.write(0, PRAMIN + 0x1_8000, 4, 0x1111_1111);
    assert_eq!(a.fb_landed, Some(0x1_8000));

    // Window at origin 1 (i.e. framebuffer 0x10000): the SAME offset must resolve 0x10000
    // higher, not onto the marker.
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 1);
    let b = p.write(0, PRAMIN + 0x1_8000, 4, 0x2222_2222);
    assert_eq!(
        b.fb_landed,
        Some(0x2_8000),
        "★ `|` would resolve this to 0x18000 and overwrite the marker"
    );

    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0);
    assert_eq!(
        p.read(0, PRAMIN + 0x1_8000, 4).value(),
        0x1111_1111,
        "the first marker must be untouched"
    );
}

/// ★★★ The whole 24-bit base is honoured — including the top of it.
///
/// A base masked to 20 bits (or computed in `u32`) wraps every high window into the first
/// few GiB of framebuffer, which is *plausible-looking* memory and therefore silent.
#[test]
fn the_base_field_is_twenty_four_bits_wide_and_none_of_it_is_truncated() {
    // Not through a plane: `SparseFb` is bounded by GA106's 12 GiB and the point of this
    // test is the addresses ABOVE it. The arithmetic is the unit under test.
    for base in [
        0u32, 1, 0xFFFF, 0x1_0000, 0x2_EFBA, 0x0F_FFFF, 0x10_0000, 0xFF_FFFF,
    ] {
        let mut w = Bar0Window::new();
        w.set_raw(base);
        assert_eq!(
            w.fb_addr(0),
            u64::from(base) << 16,
            "base {base:#x} must reach {:#x}",
            u64::from(base) << 16
        );
        assert_eq!(w.base(), base);
    }
    // …and the TARGET bits are not part of the address.
    let mut w = Bar0Window::new();
    w.set_raw((TARGET_SYS_COHERENT << TARGET_SHIFT) | 0x00FF_FFFF);
    assert_eq!(w.fb_addr(0), 0x0000_00FF_FFFF_0000);
    assert_eq!(w.target(), TARGET_SYS_COHERENT);
}

/// ★★★ **Write-then-read is total over the window, for every base and every offset.**
///
/// The property the whole rung is, quantified rather than sampled at the one address the
/// driver uses. A read path that resolved an offset differently from the write path passes
/// `the_verify_bar0_window_subtest_passes…` for any *single* offset and fails here.
#[test]
fn a_dword_written_through_the_window_reads_back_at_every_base_and_offset() {
    let p = plane();
    // Bases spread across the 24-bit field, bounded by GA106's own 12 GiB framebuffer —
    // 12 GiB >> 16 == 0x30000, so anything above that is genuinely outside the device.
    let bases = [0u32, 1, 0x10, 0xFFF, 0x1_0000, 0x2_EFBA, 0x2_FFF0];
    // Offsets: page-aligned, unaligned, straddling a store page, and at both ends of the
    // 1 MiB window.
    let offsets = [
        0u64,
        4,
        0xFFC,
        0x1000,
        0xE000,
        0x1_0000,
        0x8_0000,
        PRAMIN_LEN - 4,
    ];
    let mut expect = std::collections::HashMap::new();
    for (i, base) in bases.iter().enumerate() {
        fld_wr(&p, BASE_SHIFT, BASE_MASK, *base);
        for (j, off) in offsets.iter().enumerate() {
            let v = 0x1000_0000u64 + (i as u64) * 0x100 + j as u64;
            let w = p.write(0, PRAMIN + off, 4, v);
            let phys = w.fb_landed.expect("every one of these is inside 12 GiB");
            assert_eq!(phys, (u64::from(*base) << 16) + off);
            // ★ Later writes to an address an earlier (base, offset) pair also named must
            // win — the aliasing is real and the expectation tracks it by ADDRESS, which is
            // the only key that is well defined.
            expect.insert(phys, v);
        }
    }
    for (i, base) in bases.iter().enumerate() {
        fld_wr(&p, BASE_SHIFT, BASE_MASK, *base);
        for off in offsets {
            let phys = (u64::from(*base) << 16) + off;
            assert_eq!(
                p.read(0, PRAMIN + off, 4),
                ReadOutcome::Fb {
                    window: FbWindow::Pramin,
                    phys,
                    value: expect[&phys]
                },
                "base #{i} {base:#x} offset {off:#x}"
            );
        }
    }
}

/// ★★ **Two different windows onto one address are one byte**, which is the positive
/// statement of "one address function".
#[test]
fn two_windows_that_name_one_framebuffer_address_see_one_byte() {
    let p = plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0);
    let _ = p.write(0, PRAMIN + 0x1_0000, 4, 0xFEED_FACE);
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 1);
    assert_eq!(
        p.read(0, PRAMIN, 4).value(),
        0xFEED_FACE,
        "framebuffer 0x10000 through base 0 offset 0x10000, and through base 1 offset 0"
    );
}

/// ★ Access widths: a byte, a halfword, a dword and a quadword all round-trip, and a
/// narrower read of a wider write sees the low bytes.
#[test]
fn every_access_width_the_guest_can_use_round_trips() {
    let p = plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0x10);
    let _ = p.write(0, PRAMIN + 0x40, 8, 0x0123_4567_89AB_CDEF);
    assert_eq!(p.read(0, PRAMIN + 0x40, 8).value(), 0x0123_4567_89AB_CDEF);
    assert_eq!(p.read(0, PRAMIN + 0x40, 4).value(), 0x89AB_CDEF);
    assert_eq!(p.read(0, PRAMIN + 0x40, 2).value(), 0xCDEF);
    assert_eq!(p.read(0, PRAMIN + 0x40, 1).value(), 0xEF);
    let _ = p.write(0, PRAMIN + 0x40, 1, 0x99);
    assert_eq!(
        p.read(0, PRAMIN + 0x40, 8).value(),
        0x0123_4567_89AB_CD99,
        "a byte write must touch ONE byte"
    );
}

/// ★★ A quadword that **straddles a store page** is one access, not two half-applied ones.
#[test]
fn an_access_that_straddles_a_store_page_is_served_whole() {
    let p = plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0x20);
    let off = FB_PAGE - 4;
    let _ = p.write(0, PRAMIN + off, 8, 0xAAAA_BBBB_CCCC_DDDD);
    assert_eq!(p.read(0, PRAMIN + off, 8).value(), 0xAAAA_BBBB_CCCC_DDDD);
    // …and each half is where it should be.
    assert_eq!(p.read(0, PRAMIN + off, 4).value(), 0xCCCC_DDDD);
    assert_eq!(p.read(0, PRAMIN + off + 4, 4).value(), 0xAAAA_BBBB);
}

// =====================================================================================
// 3. NO SILENT-DROP ARM — a write that did not land says so, at the instant it happens
// =====================================================================================

/// ★★★ **With no store installed, a framebuffer write is a NAMED REFUSAL, not a success.**
///
/// This is the shape of the defect the rung was written against: a plane whose shell forgot
/// to install a framebuffer must not behave like one whose framebuffer is empty.
#[test]
fn a_framebuffer_write_with_no_store_refuses_by_name_and_never_reports_success() {
    let p = bare_plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, BENCH_BASE);
    let w = p.write(0, PRAMIN + BENCH_OFF, 4, SAMPLEDATA);
    assert!(
        w.fb_landed.is_none(),
        "⊘ nothing landed, and the outcome must not say it did"
    );
    let r = w.fb_refusal.expect("the refusal is carried WHOLE");
    assert_eq!(r.phys, BENCH_ADDR, "…with the address the window resolved");
    assert_eq!(r.len, 4);
    assert!(
        r.why.contains("set_fb"),
        "…and a wiring diagnosis: {}",
        r.why
    );
    assert!(
        w.fault.is_some(),
        "★ it is a FAULT, so the shell's unconditional fault print makes it loud in the \
         same boot rather than inferable from a later NV_ERR_MEMORY_ERROR"
    );
    let c = p.counters();
    assert_eq!(c.fb_refusals, 1);
    assert_eq!(
        c.fb_writes, 0,
        "★★ 'landed' and 'attempted' are not the same number"
    );
}

/// ★★★ An address **outside the framebuffer this chip advertises** refuses rather than
/// wrapping into a low address.
///
/// ⚠ The dangerous failure is not the refusal, it is the alternative: a store that masked
/// the address into range would put the write somewhere plausible, and every read of it
/// afterwards would agree with itself.
#[test]
fn an_address_past_the_advertised_framebuffer_refuses_instead_of_wrapping() {
    let p = plane();
    // 12 GiB >> 16 == 0x3_0000. The first window base past the end.
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0x3_0000);
    let w = p.write(0, PRAMIN, 4, 0xDEAD_BEEF);
    let r = w.fb_refusal.expect("outside the framebuffer");
    assert_eq!(r.phys, FB_LEN);
    assert_eq!(r.why, OUTSIDE_FRAMEBUFFER);
    assert!(w.fb_landed.is_none());
    // …and a read of the low address it might have wrapped onto is still zero.
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0);
    assert_eq!(p.read(0, PRAMIN, 4).value(), 0);
}

/// ★★ An access that **starts inside and ends outside** is refused whole, not truncated.
#[test]
fn an_access_that_straddles_the_end_of_the_framebuffer_is_refused_whole() {
    let mut fb = SparseFb::new(FB_LEN);
    assert!(fb.write(FB_LEN - 4, &[1, 2, 3, 4]).is_ok());
    let e = fb
        .write(FB_LEN - 4, &[1, 2, 3, 4, 5, 6, 7, 8])
        .expect_err("straddles the end");
    assert_eq!(e.why, OUTSIDE_FRAMEBUFFER);
    // ⊘ …and the first four bytes are UNCHANGED: a half-applied write is a dropped write
    // that also corrupted something.
    let mut buf = [0u8; 4];
    fb.read(FB_LEN - 4, &mut buf).expect("still readable");
    assert_eq!(buf, [1, 2, 3, 4]);
}

/// ★★ The residency ceiling is a **named refusal**, never a dropped write.
///
/// A cap that silently discarded the last page would be exactly the defect this rung
/// exists to make unrepresentable, wearing a resource-limit costume.
#[test]
fn the_residency_ceiling_refuses_by_name_rather_than_dropping_a_page() {
    // Room for two pages.
    let mut fb = SparseFb::with_cap(FB_LEN, 2 * FB_PAGE);
    fb.write(0, &[1u8; 4]).expect("first page");
    fb.write(FB_PAGE, &[2u8; 4]).expect("second page");
    assert_eq!(fb.resident_bytes(), 2 * FB_PAGE);
    // A third distinct page is refused…
    let e = fb
        .write(2 * FB_PAGE, &[3u8; 4])
        .expect_err("at the ceiling");
    assert_eq!(e.why, RESIDENT_CAP_REACHED);
    assert_eq!(e.phys, 2 * FB_PAGE);
    // …and the two pages already held still work, so the ceiling is not a wedge.
    fb.write(0, &[9u8; 4]).expect("an already-resident page");
    let mut buf = [0u8; 4];
    fb.read(0, &mut buf).expect("readable");
    assert_eq!(buf, [9u8; 4]);
    assert_eq!(
        fb.resident_bytes(),
        2 * FB_PAGE,
        "the refused page must not have been allocated"
    );
}

/// ★★ A straddling write that would cross the ceiling allocates **neither** page.
#[test]
fn a_straddling_write_at_the_ceiling_is_all_or_nothing() {
    let mut fb = SparseFb::with_cap(FB_LEN, FB_PAGE);
    let e = fb
        .write(FB_PAGE - 4, &[7u8; 8])
        .expect_err("two fresh pages, room for one");
    assert_eq!(e.why, RESIDENT_CAP_REACHED);
    assert_eq!(
        fb.resident_pages(),
        0,
        "⊘ not one page may have been allocated: a half-applied write is a dropped write \
         that also corrupted something"
    );
}

/// ★ An **unwritten** address inside the framebuffer reads zero and is `Ok` — it is memory
/// this device owns and nothing has written, which is a statement rather than an invention.
/// See `fbwin`'s module docs for why that argument does not extend to guest RAM.
#[test]
fn an_unwritten_framebuffer_address_reads_zero_rather_than_refusing() {
    let mut fb = SparseFb::new(FB_LEN);
    let mut buf = [0xFFu8; 8];

    fb.read(0x1234_5000, &mut buf).expect("inside the device");
    assert_eq!(buf, [0u8; 8]);
    assert_eq!(
        fb.resident_pages(),
        0,
        "★ and a READ must not allocate: a guest polling unwritten framebuffer would \
         otherwise grow the host's memory one page per address"
    );
}

/// ⊘ [`RefusingFb`] answers nothing, in both directions, and says why.
#[test]
fn the_default_store_refuses_both_directions_by_name() {
    let mut fb = RefusingFb;
    let mut buf = [0u8; 4];
    let FbRefused { phys, len, why } = fb.read(0x1000, &mut buf).expect_err("refuses");
    assert_eq!((phys, len), (0x1000, 4));
    assert!(why.contains("set_fb"));
    assert!(fb.write(0x1000, &[0u8; 4]).is_err());
    assert_eq!(fb.resident_bytes(), 0);
}

// =====================================================================================
// 4. The lifetime rules — a reload must not hand the next guest the last one's memory
// =====================================================================================

/// ★★★ A device reset forgets every framebuffer byte and re-points the window.
///
/// ⊘ The content is the security half: bytes that survived a device life are the previous
/// guest's page tables, instance blocks and semaphores, readable by the next one through
/// this very window, and there is no other detector for it.
#[test]
fn a_device_reset_forgets_the_framebuffer_and_re_points_the_window() {
    let p = plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, BENCH_BASE);
    fld_wr(&p, TARGET_SHIFT, TARGET_MASK, TARGET_SYS_COHERENT);
    let _ = p.write(0, PRAMIN + BENCH_OFF, 4, SAMPLEDATA);
    assert!(p.residue().fb_resident_bytes > 0);

    p.device_reset();

    assert_eq!(
        p.residue().bar0_window,
        Bar0Window::new(),
        "the window must be back at its documented reset value (_BASE_0, _TARGET_VID_MEM)"
    );
    assert_eq!(
        p.residue().fb_resident_bytes,
        0,
        "★★★ and not one byte of the previous life may remain"
    );
    // …read back through the window the previous guest used: zero, not its data.
    fld_wr(&p, BASE_SHIFT, BASE_MASK, BENCH_BASE);
    assert_eq!(p.read(0, PRAMIN + BENCH_OFF, 4).value(), 0);
}

/// ★ The framebuffer **port** survives a reset — it is the shell's wiring, like the RAM
/// port and the policy, and a device that lost it on reset would refuse every later access
/// with a wiring diagnosis nobody could act on.
#[test]
fn the_framebuffer_port_survives_a_device_reset() {
    let p = plane();
    p.device_reset();
    let w = p.write(0, PRAMIN, 4, 1);
    assert_eq!(w.fb_landed, Some(0), "the store is still installed");
}

// =====================================================================================
// 5. The chip row — the two halves are one mechanism
// =====================================================================================

/// ★★★ A chip declaring a `PRAMIN` window and **no register to move it** is refused at
/// realize.
///
/// It would otherwise serve a fixed view of framebuffer address zero while the guest
/// believed it had moved the window — every access mis-addressed, nothing logged, and the
/// first symptom hundreds of operations later. That is `#146`'s own failure, reachable
/// through a half-filled table row instead of through a bug.
#[test]
fn a_chip_with_a_window_and_no_register_to_move_it_is_refused_at_realize() {
    use kayfabe_device::ChipError;

    let mut row = ga106_copy();
    row.bar0_window_reg = 0;
    let leaked: &'static kayfabe_device::ChipProfile = Box::leak(Box::new(row));
    let e = RegPlane::new(
        leaked,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect_err("a half-filled row must not realize");
    assert!(
        matches!(e, ChipError::WindowWithoutItsRegister { .. }),
        "{e:?}"
    );
}

/// ★★ …and the mirror image: a register with no window to move.
#[test]
fn a_chip_with_a_register_and_no_window_is_refused_too() {
    use kayfabe_device::ChipError;

    let mut row = ga106_copy();
    row.pramin_window = kayfabe_device::RegSpan { base: 0, len: 0 };
    let leaked: &'static kayfabe_device::ChipProfile = Box::leak(Box::new(row));
    let e = RegPlane::new(
        leaked,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect_err("a latch nothing reads is a half-filled row too");
    assert!(
        matches!(e, ChipError::WindowWithoutItsRegister { .. }),
        "{e:?}"
    );
}

/// ★★ The window register is checked against every other read source at realize.
///
/// It is answered **before** all of them, so an overlap does not merely resolve to the
/// wrong source — it makes the other source *unreachable*.
#[test]
fn a_window_register_placed_over_another_source_is_refused_at_realize() {
    use kayfabe_device::ChipError;

    let a_boot_reg = kayfabe_device::ga10x::GA106
        .boot_regs
        .iter()
        .map(|r| r.off)
        // ⊘ Not offset zero: `0` is this row's spelling for *"this chip has no window
        // register"*, so a fixture that picked it would exercise
        // `WindowWithoutItsRegister` and silently stop testing overlap at all.
        .find(|&off| off != 0)
        .expect("GA106 declares a boot register somewhere other than offset zero");
    for (off, what) in [
        (a_boot_reg, "a boot register"),
        (kayfabe_device::ga10x::GA106.ptimer.lo_off, "the counter"),
        (
            kayfabe_device::ga10x::GA106.rom_window.base,
            "the ROM window",
        ),
        (PRAMIN + 8, "the PRAMIN window"),
        (0x0011_0100, "the GSP model"),
    ] {
        let mut row = ga106_copy();
        row.bar0_window_reg = off;
        let leaked: &'static kayfabe_device::ChipProfile = Box::leak(Box::new(row));
        let e = RegPlane::new(
            leaked,
            abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
            Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
        )
        .expect_err("an overlapping window register must be refused at realize");
        assert!(
            matches!(e, ChipError::OverlappingSources { .. }),
            "{what} at {off:#x}: {e:?}"
        );
    }
}

/// A field-for-field copy of the shipped row, so a fixture can change exactly one thing.
fn ga106_copy() -> kayfabe_device::ChipProfile {
    let g = &kayfabe_device::ga10x::GA106;
    kayfabe_device::ChipProfile {
        name: g.name,
        pci_device_id: g.pci_device_id,
        pci_revision: g.pci_revision,
        pci_subsystem_vendor_id: g.pci_subsystem_vendor_id,
        pci_subsystem_id: g.pci_subsystem_id,
        regs_aperture_len: g.regs_aperture_len,
        boot_regs: g.boot_regs,
        ptimer: g.ptimer,
        rom_window: g.rom_window,
        pramin_window: g.pramin_window,
        bar0_window_reg: g.bar0_window_reg,
        vbios_wire: g.vbios_wire,
        msix_vectors: g.msix_vectors,
        gsp_model: g.gsp_model,
        engines: g.engines,
        intr_table: g.intr_table,
        intr_subtree_map: g.intr_subtree_map,
        fb_regions: g.fb_regions,
        pci_bars: g.pci_bars,
        chip_info: g.chip_info,
        user_register_access_map: g.user_register_access_map,
        constructed_falcons: g.constructed_falcons,
        memory_system: g.memory_system,
        device_info: g.device_info,
        conf_compute: g.conf_compute,
        bif_static: g.bif_static,
        fifo_channels: g.fifo_channels,
        gmmu_static: g.gmmu_static,
        gr_static: g.gr_static,
        fb_length: g.fb_length,
    }
}

// =====================================================================================
// 6. The counters an operator reads
// =====================================================================================

/// ★★ `fb_writes` counts what **landed** and `fb_refusals` counts what did not, and the two
/// never absorb each other.
#[test]
fn landed_and_refused_are_two_numbers_and_neither_absorbs_the_other() {
    let p = plane();
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0);
    let _ = p.write(0, PRAMIN, 4, 1);
    let _ = p.write(0, PRAMIN + 4, 4, 2);
    fld_wr(&p, BASE_SHIFT, BASE_MASK, 0x3_0000); // the first base past the end
    let _ = p.write(0, PRAMIN, 4, 3);

    let c = p.counters();
    assert_eq!(c.fb_writes, 2);
    assert_eq!(c.fb_refusals, 1);
    assert_eq!(
        c.fb_window_writes, 0,
        "⊘ the BAR0 window is never reported as a dropped translated window"
    );
    assert_eq!(c.unclaimed_writes, 0);
    assert!(c.bar0_window_writes >= 2 && c.bar0_window_reads >= 2);
}

/// ★★★ **UPDATED DELIBERATELY by `#149`, and the two halves went different ways.**
///
/// This test used to say *"the two GMMU-translated windows are unchanged — they still have
/// no address model"*. One of them still has none and the other now does, and collapsing
/// that into one sentence would hide the whole rung:
///
/// - **The framebuffer aperture (BAR1)** is unchanged. Nothing in this port resolves an
///   access through it, so [`ReadOutcome::FbWindow`] — *"no address model at all"* — is
///   still the honest answer and the assertion below is untouched.
/// - **The instance/BAR2 window** now HAS one. A plane with no page-table format installed
///   therefore answers a different thing: a **named refusal** carrying the virtual address
///   ([`ReadOutcome::TranslationRefused`], [`NO_MMU_PORT`]), which is the same shape
///   `RefusingFb` and `RefusingRam` already use for *"the shell never wired this port"*.
///   That distinction is worth a test on its own, because *"this port cannot translate"*
///   and *"this device was built without a format"* are different findings and only the
///   second one is a wiring bug.
#[test]
fn bar1_has_no_address_model_and_bar2_now_refuses_by_name_instead() {
    let p = plane();
    assert_eq!(
        p.read(1, 0x0009_008C, 4),
        ReadOutcome::FbWindow(FbWindow::FbAperture),
        "⊘ BAR1 is unchanged: no address model, and serving it would invent a translation"
    );
    assert!(
        matches!(
            p.read(2, 0x0000_1000, 4),
            ReadOutcome::TranslationRefused {
                window: FbWindow::InstanceWindow,
                va: 0x0000_1000,
                why,
            } if why == kayfabe_device::plane::NO_MMU_PORT
        ),
        "BAR2 must name the missing format and carry the VIRTUAL address, never read as \
         a plausible zero"
    );
    let w = p.write(2, 0x0000_1000, 4, 1);
    assert_eq!(
        w.fb_window, None,
        "⊘ BAR2 is no longer a 'dropped window' — it is a named translation refusal"
    );
    assert!(w.fb_landed.is_none() && w.fb_refusal.is_none());
    let r = w
        .bar2_refusal
        .expect("a translated write that did not land says so");
    assert_eq!(r.va, 0x0000_1000);
    assert_eq!(r.len, 4);
    assert_eq!(r.why, kayfabe_device::plane::NO_MMU_PORT);
    assert_eq!(
        w.fault,
        Some(kayfabe_device::plane::BAR2_WRITE_REFUSED),
        "★ and it is a FAULT, so the shell prints it at the instant the bytes are lost"
    );
    let c = p.counters();
    assert_eq!(c.fb_window_writes, 0, "the BAR1 arm did not fire");
    assert_eq!(
        c.bar2_faults, 2,
        "one read and one write, both refused by name"
    );
    assert_eq!(c.fb_reads, 0);
    assert_eq!(c.bar2_reads, 0);
    assert_eq!(c.bar2_writes, 0);
}
