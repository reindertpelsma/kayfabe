// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ P1+P2 inc C (`docs/design/V3_P1P2_TSPACE.md` §3.8) — **THE T-SPACE ADDRESS PERIMETER.**
//!
//! `OWNER_RULINGS.md` §R(b): code that produces an address hardware will dereference is perimeter
//! material even though it compiles as safe Rust. This file holds ALL of it for the Translated
//! plane, and nothing else:
//!
//! - the two windows' bounds ([`TWindows`]: the store window `[0, carve)` — never the firmware
//!   carve-out — and the guest-RAM window, both ending at or below the ring region, below 2^40);
//! - [`WindowAddr`], an address inside one of them, which ONLY this file can construct, and only
//!   after the bound check;
//! - the ONLY functions that put an address word or a `LAUNCH_DMA` word into the output (the
//!   `put_*` functions below take a [`WindowAddr`], never a `u64`);
//! - the ring's own address authoring: its completion tail ([`fence_words`]) and its GP entry
//!   ([`ring_gp_entry`]).
//!
//! ★ No `unsafe` block: none is needed. The file is named `*_unsafe.rs` because §R makes it the
//! audit perimeter ("this file can violate memory safety" — here: the GPU's), so every export
//! validates its own inputs and a buggy safe caller cannot make it emit an address outside a
//! window. The gate-3 table of the exports and the test showing each check is
//! `docs/design/V3_P1P2_TSPACE.md` §3.8.

use crate::ttables::{Launch, Tier, encode_launch};
use kf_abi::submit::{ce, fifo, gp_entry, method_header_inc};

/// Every GPU address a T-space channel may name is below this: a GP entry's `GET_HI` and the legacy
/// host semaphore's `SEMAPHOREA` are 8 bits (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h`).
pub const VA_LIMIT_40: u64 = 1 << 40;

/// ★ The two windows of the T-space, bounded. Constructed only through [`TWindows::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TWindows {
    fb_base: u64,
    fb_len: u64,
    ram_base: u64,
    ram_len: u64,
}

/// ★ An address inside one of the two windows. Only this file constructs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowAddr(u64);

impl WindowAddr {
    /// The address.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TWindows {
    /// The store window `(base, len)` — `len` is the carve-out's base, so the window never covers
    /// it — and the guest-RAM window `(base, len)`. `limit` is where the ring region starts.
    ///
    /// # Errors
    /// A window that is empty, overflows, or ends past `limit`; a `limit` above 2^40.
    pub fn new(fb: (u64, u64), ram: (u64, u64), limit: u64) -> Result<TWindows, String> {
        if limit > VA_LIMIT_40 {
            return Err(format!("limit {limit:#x} is above 2^40"));
        }
        for (what, (base, len)) in [("store", fb), ("guest-RAM", ram)] {
            let end = base
                .checked_add(len)
                .filter(|_| len > 0)
                .ok_or_else(|| format!("{what} window {base:#x}+{len:#x} is empty or overflows"))?;
            if end > limit {
                return Err(format!("{what} window ends at {end:#x}, past {limit:#x}"));
            }
        }
        Ok(TWindows {
            fb_base: fb.0,
            fb_len: fb.1,
            ram_base: ram.0,
            ram_len: ram.1,
        })
    }

    /// Guest VRAM `[off, off+len)` in the store window — `None` unless it lies wholly below the
    /// carve-out (`len > 0`).
    #[must_use]
    pub fn fb(&self, off: u64, len: u64) -> Option<WindowAddr> {
        let end = off.checked_add(len).filter(|_| len > 0)?;
        (end <= self.fb_len).then(|| WindowAddr(self.fb_base + off))
    }

    /// Guest RAM at memfd offset `[off, off+len)` in the RAM window — `None` unless it lies wholly
    /// inside it (`len > 0`).
    #[must_use]
    pub fn ram(&self, off: u64, len: u64) -> Option<WindowAddr> {
        let end = off.checked_add(len).filter(|_| len > 0)?;
        (end <= self.ram_len).then(|| WindowAddr(self.ram_base + off))
    }

    /// `[va, va+len)` lies wholly inside one window — what every emitted address must satisfy
    /// (the property tests decode the output and ask this).
    #[must_use]
    pub fn contains(&self, va: u64, len: u64) -> bool {
        let Some(end) = va.checked_add(len) else {
            return false;
        };
        (va >= self.fb_base && end <= self.fb_base + self.fb_len)
            || (va >= self.ram_base && end <= self.ram_base + self.ram_len)
    }
}

fn put(out: &mut Vec<u32>, sub: u32, m: u32, v: u32) {
    // `method_header_inc` refuses only an out-of-range method or subchannel; every caller below
    // passes a class constant and a subchannel the decoder accepted (0-7).
    if let Some(h) = method_header_inc(sub & 7, m, 1) {
        out.push(h);
        out.push(v);
    }
}

/// Which CE offset pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeSide {
    /// `OFFSET_IN_UPPER/LOWER`.
    In,
    /// `OFFSET_OUT_UPPER/LOWER`.
    Out,
}

/// ★ Emit a CE operand address: `OFFSET_{IN,OUT}_UPPER` (masked to the tier's field) then `_LOWER`.
pub fn put_ce_offset(out: &mut Vec<u32>, sub: u32, tier: Tier, side: CeSide, a: WindowAddr) {
    let upper = match side {
        CeSide::In => ce::OFFSET_IN_UPPER,
        CeSide::Out => ce::OFFSET_OUT_UPPER,
    };
    put(out, sub, upper, ((a.0 >> 32) as u32) & tier.upper_mask());
    put(out, sub, upper + 4, (a.0 & 0xFFFF_FFFF) as u32);
}

/// ★ Emit a CE semaphore address: `SET_SEMAPHORE_A` (masked) then `SET_SEMAPHORE_B`.
pub fn put_ce_semaphore(out: &mut Vec<u32>, sub: u32, tier: Tier, a: WindowAddr) {
    put(
        out,
        sub,
        ce::SET_SEMAPHORE_A,
        ((a.0 >> 32) as u32) & tier.upper_mask(),
    );
    put(out, sub, ce::SET_SEMAPHORE_B, (a.0 & 0xFFFF_FFFF) as u32);
}

/// ★ Emit the legacy host semaphore address: `SEMAPHOREA` (39:32) then `SEMAPHOREB` (31:2).
///
/// # Errors
/// The address is at or above 2^40, or not 4-byte aligned: the 40-bit form would truncate it, and
/// an engine that wrote a truncated address would write somewhere no rewriter chose.
pub fn put_host_semaphore(out: &mut Vec<u32>, sub: u32, a: WindowAddr) -> Result<(), u64> {
    if a.0 >= VA_LIMIT_40 || a.0 & 3 != 0 {
        return Err(a.0);
    }
    put(out, sub, HOST_SEMAPHORE_A, ((a.0 >> 32) & 0xFF) as u32);
    put(out, sub, HOST_SEMAPHORE_B, (a.0 & 0xFFFF_FFFC) as u32);
    Ok(())
}

/// ★ Emit the host `SEM_ADDR_LO/HI`: 57 bits on a `wide` tier, 40 bits before.
///
/// # Errors
/// As [`put_host_semaphore`] (the 40-bit limit applies only when not `wide`).
pub fn put_host_sem_addr(
    out: &mut Vec<u32>,
    sub: u32,
    a: WindowAddr,
    wide: bool,
) -> Result<(), u64> {
    let (limit, hi_mask) = if wide {
        (1u64 << 57, 0x1FF_FFFF)
    } else {
        (VA_LIMIT_40, 0xFF)
    };
    if a.0 >= limit || a.0 & 3 != 0 {
        return Err(a.0);
    }
    put(out, sub, fifo::SEM_ADDR_LO, (a.0 & 0xFFFF_FFFC) as u32);
    put(out, sub, fifo::SEM_ADDR_HI, ((a.0 >> 32) as u32) & hi_mask);
    Ok(())
}

/// ★ Emit `LAUNCH_DMA` authored from `l` — both operands VIRTUAL, every unnamed bit zero.
pub fn put_launch(out: &mut Vec<u32>, sub: u32, l: &Launch) {
    put(out, sub, ce::LAUNCH_DMA, encode_launch(l));
}

/// `NV906F_SEMAPHOREA` / `_B` (`ogkm-580: src/common/sdk/nvidia/inc/class/cl906f.h:78-81`).
const HOST_SEMAPHORE_A: u32 = 0x10;
const HOST_SEMAPHORE_B: u32 = 0x14;

/// ★ NVIDIA's completion tail for our own ring: a host release of `payload` at `fence_va` with
/// `RELEASE_WFI`, then the host `NON_STALL_INTERRUPT` (`nvidia-push.c:1047-1059`). `None` unless
/// `fence_va` is 4-byte aligned and below 2^40 (`SEM_ADDR_HI` is 8 bits on Turing–Ada).
#[must_use]
pub fn fence_words(fence_va: u64, payload: u32) -> Option<Vec<u32>> {
    if fence_va >= VA_LIMIT_40 || fence_va & 3 != 0 {
        return None;
    }
    Some(vec![
        method_header_inc(0, fifo::SEM_ADDR_LO, 5)?,
        (fence_va & 0xFFFF_FFFC) as u32,
        ((fence_va >> 32) & 0xFF) as u32,
        payload,
        0,
        fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN,
        method_header_inc(0, fifo::NON_STALL_INTERRUPT, 1)?,
        0,
    ])
}

/// ★ The GP entry of our own ring for `len` bytes at pushbuffer offset `start`: `None` unless the
/// range lies inside the pushbuffer (`[0, pb_bytes)`) and the address is below 2^40.
#[must_use]
pub fn ring_gp_entry(ring_va: u64, pb_bytes: u64, start: u64, len: u64) -> Option<u64> {
    let end = start.checked_add(len)?;
    if end > pb_bytes {
        return None;
    }
    gp_entry(ring_va.checked_add(start)?, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: (u64, u64) = (0x1_2000_0000, 0x2_EFBE_0000);
    const R: (u64, u64) = (0x4_1000_0000, 0x2_0000_0000);
    const LIMIT: u64 = VA_LIMIT_40 - (4 << 30);

    /// ★ §3.8 gate-3 row `TWindows::new`: refuses an empty, overflowing or too-high window.
    #[test]
    fn windows_are_bounded_at_construction() {
        assert!(TWindows::new(W, R, LIMIT).is_ok());
        assert!(TWindows::new((W.0, 0), R, LIMIT).is_err(), "empty");
        assert!(TWindows::new((u64::MAX, 2), R, LIMIT).is_err(), "overflow");
        assert!(
            TWindows::new(W, (LIMIT - 0x1000, 0x2000), LIMIT).is_err(),
            "past the limit"
        );
        assert!(
            TWindows::new(W, R, VA_LIMIT_40 + 1).is_err(),
            "limit above 2^40"
        );
    }

    /// ★ Gate-3 rows `fb` / `ram`: an address is handed out only for a range wholly inside its
    /// window; the store window ends at the carve-out.
    #[test]
    fn a_window_address_is_only_ever_inside_its_window() {
        let w = TWindows::new(W, R, LIMIT).unwrap();
        assert_eq!(w.fb(0, 4).map(WindowAddr::get), Some(W.0));
        assert_eq!(w.fb(W.1 - 8, 8).map(WindowAddr::get), Some(W.0 + W.1 - 8));
        for (off, len) in [(W.1, 4), (W.1 - 4, 8), (0, 0), (u64::MAX, 2)] {
            assert_eq!(w.fb(off, len), None, "fb {off:#x}+{len:#x}");
        }
        assert_eq!(w.ram(R.1 - 4, 4).map(WindowAddr::get), Some(R.0 + R.1 - 4));
        assert_eq!(w.ram(R.1, 1), None);
        assert!(w.contains(W.0, W.1) && w.contains(R.0, R.1));
        assert!(!w.contains(W.0 + W.1, 1) && !w.contains(R.0 - 1, 2));
    }

    /// ★ Gate-3 rows `put_host_semaphore` / `put_host_sem_addr` / `fence_words`: the 40-bit forms
    /// refuse an address at or above 2^40; the wide form takes 57 bits.
    #[test]
    fn the_forty_bit_forms_refuse_what_they_would_truncate() {
        let mut out = Vec::new();
        assert_eq!(
            put_host_semaphore(&mut out, 0, WindowAddr(VA_LIMIT_40)),
            Err(VA_LIMIT_40)
        );
        assert_eq!(
            put_host_sem_addr(&mut out, 0, WindowAddr(VA_LIMIT_40), false),
            Err(VA_LIMIT_40)
        );
        assert!(out.is_empty(), "a refusal emits nothing");
        assert_eq!(
            put_host_sem_addr(&mut out, 0, WindowAddr(VA_LIMIT_40), true),
            Ok(())
        );
        assert_eq!(
            put_host_semaphore(&mut out, 0, WindowAddr(0x1002)),
            Err(0x1002)
        );
        assert!(fence_words(VA_LIMIT_40, 1).is_none());
        assert!(fence_words(0xF_FFFF_F000, 1).is_some());
    }

    #[test]
    fn the_ring_gp_entry_stays_in_the_pushbuffer() {
        assert!(ring_gp_entry(0xFF_0000_0000, 0xD_0000, 0, 0x100).is_some());
        assert!(ring_gp_entry(0xFF_0000_0000, 0xD_0000, 0xC_FF00, 0x200).is_none());
        assert!(ring_gp_entry(VA_LIMIT_40, 0xD_0000, 0, 0x100).is_none());
    }
}
