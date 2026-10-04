// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ P1+P2 inc C (`docs/design/V3_P1P2_TSPACE.md` §3.8) — **THE T-SPACE ADDRESS PERIMETER.**
//!
//! `OWNER_RULINGS.md` §R(b): code that produces an address hardware will dereference is perimeter
//! material even though it compiles as safe Rust. This file holds ALL of it for the Translated
//! plane, and nothing else:
//!
//! - the two windows' bounds ([`TWindows`]: the store window `[0, carve)` — never the firmware
//!   carve-out — and the guest-RAM window, both ending at or below the ring region, below 2^40);
//! - [`WindowAddr`], an address inside one of them together with the bytes it was validated for,
//!   which ONLY this file can construct, and only after the bound check;
//! - the ONLY functions that put an address word, a footprint register or a trigger word
//!   (`LAUNCH_DMA`, `SEMAPHORED`, `SEM_EXECUTE`) into the output (the `put_*` functions below take a
//!   [`WindowAddr`], never a `u64`). ★ Review fix 2026-10-04: each trigger RECOMPUTES the engine's
//!   footprint from the very registers it emits — `LINE_LENGTH_IN` × element, `PITCH` ×
//!   (`LINE_COUNT` − 1), the remap components, the semaphore's size from its own word — and refuses
//!   it unless it fits the bytes every operand's [`WindowAddr`] was validated for. So the bound no
//!   longer depends on arithmetic at the call site: a buggy safe caller that under-sizes an operand
//!   gets a refusal, never an engine access past a resolved row or past a window;
//! - the ring's own address authoring: its completion tail ([`fence_words`]) and its GP entry
//!   ([`ring_gp_entry`]).
//!
//! ★ No `unsafe` block: none is needed. The file is named `*_unsafe.rs` because §R makes it the
//! audit perimeter ("this file can violate memory safety" — here: the GPU's), so every export
//! validates its own inputs and a buggy safe caller cannot make it emit an address outside a
//! window. The gate-3 table of the exports and the test showing each check is
//! `docs/design/V3_P1P2_TSPACE.md` §3.8.

use crate::ttables::{Launch, REMAP_NAMED, SemExecute, SemaphoreD, Tier, encode_launch};
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

/// ★ An address inside one of the two windows, and the bytes `[addr, addr+bytes)` that were checked
/// to lie inside it (and, for a resolved operand, inside one placement row). Only this file
/// constructs one; every trigger refuses an access that would reach past `bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowAddr {
    addr: u64,
    bytes: u64,
}

impl WindowAddr {
    /// The address.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.addr
    }

    /// The bytes it was validated for.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.bytes
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
        (end <= self.fb_len).then(|| WindowAddr {
            addr: self.fb_base + off,
            bytes: len,
        })
    }

    /// Guest RAM at memfd offset `[off, off+len)` in the RAM window — `None` unless it lies wholly
    /// inside it (`len > 0`).
    #[must_use]
    pub fn ram(&self, off: u64, len: u64) -> Option<WindowAddr> {
        let end = off.checked_add(len).filter(|_| len > 0)?;
        (end <= self.ram_len).then(|| WindowAddr {
            addr: self.ram_base + off,
            bytes: len,
        })
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

/// ★ Why the perimeter refused a trigger — nothing was emitted for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerimeterRefusal {
    /// A semaphore address a 40-bit form (`SEMAPHOREA`, a pre-Hopper `SEM_ADDR_HI`, the fence)
    /// would truncate, or one below 4-byte alignment.
    Sem40 {
        /// The address.
        va: u64,
    },
    /// The access the emitted registers describe reaches past the bytes its operand was validated
    /// for.
    Footprint {
        /// Which operand: `"src"`, `"dst"` or `"semaphore"`.
        side: &'static str,
        /// Bytes the engine would touch.
        need: u64,
        /// Bytes validated.
        have: u64,
    },
    /// An operand the launch reads or writes has no address, or one is given that it never
    /// touches.
    Operands {
        /// What is inconsistent.
        what: &'static str,
    },
    /// A field the footprint cannot be computed for: a block-linear side, an unnamed bit in
    /// `SET_REMAP_COMPONENTS` or `REQ_ATTR`, an extent that overflows.
    Field {
        /// What was refused.
        what: &'static str,
    },
}

/// The footprint registers a launch piece runs with. [`put_launch_piece`] emits every one the
/// engine reads for the launch and computes the footprint from exactly those values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PieceRegs {
    /// `LINE_LENGTH_IN` (elements with remap, else bytes).
    pub line_len: u32,
    /// `LINE_COUNT` (read with `MULTI_LINE_ENABLE`).
    pub line_count: u32,
    /// `PITCH_IN` (read with `MULTI_LINE_ENABLE`).
    pub pitch_in: u32,
    /// `PITCH_OUT` (read with `MULTI_LINE_ENABLE`).
    pub pitch_out: u32,
    /// `SET_REMAP_COMPONENTS` (read with `REMAP_ENABLE`).
    pub remap: u32,
    /// `SET_REMAP_CONST_A`.
    pub const_a: u32,
    /// `SET_REMAP_CONST_B`.
    pub const_b: u32,
    /// `REQ_ATTR` (`CAB5`), when the guest set one.
    pub req_attr: Option<u32>,
}

/// A CE semaphore release of one launch piece: where, and its payload words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PieceSema {
    /// The release address, validated for its bytes.
    pub at: WindowAddr,
    /// `SET_SEMAPHORE_PAYLOAD`.
    pub payload: u32,
    /// `SET_SEMAPHORE_PAYLOAD_UPPER` — present exactly when the launch's payload is two words.
    pub upper: Option<u32>,
}

/// ★ One launch piece as T-mode authors it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchPiece {
    /// The subchannel.
    pub sub: u32,
    /// The bound CE class's tier.
    pub tier: Tier,
    /// The launch word's fields (both operands are emitted VIRTUAL).
    pub launch: Launch,
    /// The footprint registers.
    pub regs: PieceRegs,
    /// The source, present exactly when the engine reads one.
    pub src: Option<WindowAddr>,
    /// The destination, present exactly when data moves.
    pub dst: Option<WindowAddr>,
    /// The release, present exactly when `SEMAPHORE_TYPE` asks for one.
    pub sema: Option<PieceSema>,
}

/// `(source element bytes, destination element bytes, the destination reads the source)` of a
/// remap word — derived here from the header's fields, independently of the decoder.
const fn remap_shape(remap: u32) -> (u64, u64, bool) {
    let comp = ((remap >> 16) & 0x3) as u64 + 1;
    let n_src = ((remap >> 20) & 0x3) as u64 + 1;
    let n_dst = ((remap >> 24) & 0x3) + 1;
    let mut reads = false;
    let mut c = 0;
    while c < n_dst {
        // `DST_*` values 0-3 select a SOURCE component (`clc7b5.h:183-186`).
        if (remap >> (4 * c)) & 0x7 <= 3 {
            reads = true;
        }
        c += 1;
    }
    (comp * n_src, comp * n_dst as u64, reads)
}

/// Bytes a side touches: `LINE_LENGTH_IN × element` for one line, `PITCH × (LINE_COUNT − 1) + that`
/// for several. `None` on overflow.
fn extent(line_len: u32, elem: u64, multi: bool, line_count: u32, pitch: u32) -> Option<u64> {
    let line = u64::from(line_len).checked_mul(elem)?;
    if !multi || line_count <= 1 {
        return Some(line);
    }
    u64::from(pitch)
        .checked_mul(u64::from(line_count) - 1)?
        .checked_add(line)
}

fn fits(side: &'static str, need: u64, a: WindowAddr) -> Result<(), PerimeterRefusal> {
    if need > a.bytes {
        return Err(PerimeterRefusal::Footprint {
            side,
            need,
            have: a.bytes,
        });
    }
    Ok(())
}

/// ★★ **Emit one launch piece — its footprint registers, its addresses and its `LAUNCH_DMA` — and
/// nothing unless the footprint the engine will use fits every operand.** The footprint is computed
/// HERE from the registers this function emits (never taken from the caller): the source extent
/// from `LINE_LENGTH_IN`, the source element (remap `COMPONENT_SIZE × NUM_SRC_COMPONENTS`, else 1),
/// `PITCH_IN` and `LINE_COUNT`; the destination's likewise with `NUM_DST_COMPONENTS` and
/// `PITCH_OUT`; the release's size from `SEMAPHORE_TYPE` and `SEMAPHORE_PAYLOAD_SIZE` (16 bytes for
/// a four-word / timestamped release, 8 for a two-word payload, else 4).
///
/// # Errors
/// [`PerimeterRefusal`] — nothing is emitted.
pub fn put_launch_piece(out: &mut Vec<u32>, p: &LaunchPiece) -> Result<(), PerimeterRefusal> {
    let l = &p.launch;
    let r = &p.regs;
    if r.remap & !REMAP_NAMED != 0 {
        return Err(PerimeterRefusal::Field {
            what: "SET_REMAP_COMPONENTS beyond its named fields",
        });
    }
    if r.req_attr.is_some_and(|v| v & !0x3 != 0) {
        return Err(PerimeterRefusal::Field {
            what: "REQ_ATTR beyond PREFETCH_L2_CLASS (1:0)",
        });
    }
    let moves = l.transfer != 0;
    let (src_elem, dst_elem, remap_reads) = if l.remap {
        remap_shape(r.remap)
    } else {
        (1, 1, true)
    };
    let reads_src = moves && remap_reads;
    if (reads_src && !l.src_pitch) || (moves && !l.dst_pitch) {
        return Err(PerimeterRefusal::Field {
            what: "a block-linear side (its footprint is not modelled)",
        });
    }
    if p.src.is_some() != reads_src || p.dst.is_some() != moves {
        return Err(PerimeterRefusal::Operands {
            what: "a source or destination the launch does not touch, or a touched one missing",
        });
    }
    if p.sema.is_some() != (l.sema != 0)
        || p.sema
            .is_some_and(|s| s.upper.is_some() != l.payload_two_word)
    {
        return Err(PerimeterRefusal::Operands {
            what: "a release the launch does not make, or a made one missing",
        });
    }
    let overflow = PerimeterRefusal::Field {
        what: "an extent that overflows",
    };
    if let Some(a) = p.src {
        let need =
            extent(r.line_len, src_elem, l.multi_line, r.line_count, r.pitch_in).ok_or(overflow)?;
        fits("src", need, a)?;
    }
    if let Some(a) = p.dst {
        let need = extent(
            r.line_len,
            dst_elem,
            l.multi_line,
            r.line_count,
            r.pitch_out,
        )
        .ok_or(overflow)?;
        fits("dst", need, a)?;
    }
    if let Some(s) = p.sema {
        let need = if l.sema == 2 {
            16
        } else if l.payload_two_word {
            8
        } else {
            4
        };
        fits("semaphore", need, s.at)?;
    }
    let sub = p.sub;
    let mut w = Vec::new();
    put(&mut w, sub, ce::LINE_LENGTH_IN, r.line_len);
    if l.multi_line {
        put(&mut w, sub, ce::LINE_COUNT, r.line_count);
        put(&mut w, sub, CE_PITCH_IN, r.pitch_in);
        put(&mut w, sub, CE_PITCH_OUT, r.pitch_out);
    }
    if l.remap {
        put(&mut w, sub, ce::SET_REMAP_CONST_A, r.const_a);
        put(&mut w, sub, CE_SET_REMAP_CONST_B, r.const_b);
        put(&mut w, sub, ce::SET_REMAP_COMPONENTS, r.remap & REMAP_NAMED);
    }
    if let Some(v) = r.req_attr {
        put(&mut w, sub, CE_REQ_ATTR, v & 0x3);
    }
    if let Some(s) = p.sema {
        put(
            &mut w,
            sub,
            ce::SET_SEMAPHORE_A,
            ((s.at.addr >> 32) as u32) & p.tier.upper_mask(),
        );
        put(&mut w, sub, ce::SET_SEMAPHORE_A + 4, s.at.addr as u32);
        put(&mut w, sub, ce::SET_SEMAPHORE_PAYLOAD, s.payload);
        if let Some(u) = s.upper {
            put(&mut w, sub, ce::SET_SEMAPHORE_PAYLOAD_UPPER, u);
        }
    }
    for (side, a) in [(CeSide::In, p.src), (CeSide::Out, p.dst)] {
        let Some(a) = a else { continue };
        let upper = match side {
            CeSide::In => ce::OFFSET_IN_UPPER,
            CeSide::Out => ce::OFFSET_OUT_UPPER,
        };
        put(
            &mut w,
            sub,
            upper,
            ((a.addr >> 32) as u32) & p.tier.upper_mask(),
        );
        put(&mut w, sub, upper + 4, (a.addr & 0xFFFF_FFFF) as u32);
    }
    put(&mut w, sub, ce::LAUNCH_DMA, encode_launch(l));
    out.extend(w);
    Ok(())
}

/// `NVC7B5_PITCH_IN` / `_OUT`, `NVC7B5_SET_REMAP_CONST_B`, `NVCAB5_REQ_ATTR`
/// (`clc7b5.h`, `clcab5.h:29-41`).
const CE_PITCH_IN: u32 = 0x410;
const CE_PITCH_OUT: u32 = 0x414;
const CE_SET_REMAP_CONST_B: u32 = 0x704;
const CE_REQ_ATTR: u32 = 0x754;

/// The bytes a host `SEMAPHORED` word touches — from the word itself (`clc56f.h:83-107`):
/// `OPERATION` (4:0) RELEASE (2) without `RELEASE_SIZE` (24) 4BYTE is a 16-byte release (payload +
/// timestamp); every other operation reads or updates 4 bytes.
const fn semaphored_bytes(word: u32) -> u64 {
    if word & 0x1F == 2 && (word >> 24) & 1 == 0 {
        16
    } else {
        4
    }
}

/// The bytes a host `SEM_EXECUTE` word touches — from the word itself (`clc56f.h:214-244`):
/// `OPERATION` (2:0) RELEASE (1) with `RELEASE_TIMESTAMP` (25) writes 16; else `PAYLOAD_SIZE`
/// (24) 64BIT touches 8; else 4.
const fn sem_execute_bytes(word: u32) -> u64 {
    if word & 0x7 == 1 && (word >> 25) & 1 != 0 {
        16
    } else if (word >> 24) & 1 != 0 {
        8
    } else {
        4
    }
}

/// ★ Emit a legacy host semaphore operation: `SEMAPHOREA` (39:32), `SEMAPHOREB` (31:2), the payload
/// `SEMAPHOREC` and the operation `SEMAPHORED` — refused unless the bytes the operation touches
/// (from the word emitted) fit the address's validated bytes.
///
/// # Errors
/// [`PerimeterRefusal::Sem40`] at or above 2^40 or unaligned (the 40-bit form would truncate it,
/// and an engine that wrote a truncated address would write somewhere no rewriter chose);
/// [`PerimeterRefusal::Footprint`]. Nothing is emitted.
pub fn put_host_semaphore(
    out: &mut Vec<u32>,
    sub: u32,
    a: WindowAddr,
    payload: u32,
    op: &SemaphoreD,
) -> Result<(), PerimeterRefusal> {
    if a.addr >= VA_LIMIT_40 || a.addr & 3 != 0 {
        return Err(PerimeterRefusal::Sem40 { va: a.addr });
    }
    let word = op.encode();
    fits("semaphore", semaphored_bytes(word), a)?;
    put(out, sub, HOST_SEMAPHORE_A, ((a.addr >> 32) & 0xFF) as u32);
    put(out, sub, HOST_SEMAPHORE_B, (a.addr & 0xFFFF_FFFC) as u32);
    put(out, sub, HOST_SEMAPHORE_C, payload);
    put(out, sub, HOST_SEMAPHORE_D, word);
    Ok(())
}

/// ★ Emit a host `SEM_ADDR_LO/HI` + `SEM_PAYLOAD_LO/HI` + `SEM_EXECUTE`: 57-bit `SEM_ADDR_HI` on a
/// `wide` tier, 40-bit before — refused unless the bytes the operation touches (from the word
/// emitted) fit the address's validated bytes.
///
/// # Errors
/// As [`put_host_semaphore`] (the 40-bit limit applies only when not `wide`).
pub fn put_host_sem_execute(
    out: &mut Vec<u32>,
    sub: u32,
    a: WindowAddr,
    payload: (u32, u32),
    op: &SemExecute,
    wide: bool,
) -> Result<(), PerimeterRefusal> {
    let (limit, hi_mask) = if wide {
        (1u64 << 57, 0x1FF_FFFF)
    } else {
        (VA_LIMIT_40, 0xFF)
    };
    if a.addr >= limit || a.addr & 3 != 0 {
        return Err(PerimeterRefusal::Sem40 { va: a.addr });
    }
    let word = op.encode();
    fits("semaphore", sem_execute_bytes(word), a)?;
    put(out, sub, fifo::SEM_ADDR_LO, (a.addr & 0xFFFF_FFFC) as u32);
    put(
        out,
        sub,
        fifo::SEM_ADDR_HI,
        ((a.addr >> 32) as u32) & hi_mask,
    );
    put(out, sub, fifo::SEM_PAYLOAD_LO, payload.0);
    put(out, sub, fifo::SEM_PAYLOAD_HI, payload.1);
    put(out, sub, fifo::SEM_EXECUTE, word);
    Ok(())
}

/// `NV906F_SEMAPHOREA` … `_D` (`ogkm-580: src/common/sdk/nvidia/inc/class/cl906f.h:78-81`).
const HOST_SEMAPHORE_A: u32 = 0x10;
const HOST_SEMAPHORE_B: u32 = 0x14;
const HOST_SEMAPHORE_C: u32 = 0x18;
const HOST_SEMAPHORE_D: u32 = 0x1C;

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

    fn at(addr: u64, bytes: u64) -> WindowAddr {
        WindowAddr { addr, bytes }
    }

    /// ★ Gate-3 rows `put_host_semaphore` / `put_host_sem_execute` / `fence_words`: the 40-bit
    /// forms refuse an address at or above 2^40; the wide form takes 57 bits.
    #[test]
    fn the_forty_bit_forms_refuse_what_they_would_truncate() {
        let rel4 = SemaphoreD::decode(2 | (1 << 24)).unwrap(); // RELEASE, 4BYTE
        let rel = SemExecute::decode(1).unwrap(); // RELEASE, 32-bit
        let mut out = Vec::new();
        assert_eq!(
            put_host_semaphore(&mut out, 0, at(VA_LIMIT_40, 4), 1, &rel4),
            Err(PerimeterRefusal::Sem40 { va: VA_LIMIT_40 })
        );
        assert_eq!(
            put_host_sem_execute(&mut out, 0, at(VA_LIMIT_40, 4), (1, 0), &rel, false),
            Err(PerimeterRefusal::Sem40 { va: VA_LIMIT_40 })
        );
        assert!(out.is_empty(), "a refusal emits nothing");
        assert_eq!(
            put_host_sem_execute(&mut out, 0, at(VA_LIMIT_40, 4), (1, 0), &rel, true),
            Ok(())
        );
        assert_eq!(
            put_host_semaphore(&mut out, 0, at(0x1002, 4), 1, &rel4),
            Err(PerimeterRefusal::Sem40 { va: 0x1002 })
        );
        assert!(fence_words(VA_LIMIT_40, 1).is_none());
        assert!(fence_words(0xF_FFFF_F000, 1).is_some());
    }

    /// ★★ Gate-3 rows `put_host_semaphore` / `put_host_sem_execute` (review fix 2026-10-04): the
    /// bytes an operation touches come from the word EMITTED — a 16-byte legacy release, a 16-byte
    /// timestamped `SEM_EXECUTE` release, an 8-byte 64-bit payload — and must fit the address's
    /// validated bytes, or nothing is emitted.
    #[test]
    fn a_semaphore_footprint_must_fit_its_validated_bytes() {
        let rel16 = SemaphoreD::decode(2).unwrap(); // RELEASE, 16BYTE
        let acq = SemaphoreD::decode(1).unwrap(); // ACQUIRE: 4 bytes
        let ts = SemExecute::decode(1 | (1 << 25)).unwrap(); // RELEASE + TIMESTAMP
        let p64 = SemExecute::decode(1 | (1 << 24)).unwrap(); // RELEASE, 64-bit payload
        let mut out = Vec::new();
        assert_eq!(
            put_host_semaphore(&mut out, 0, at(0x1000, 4), 1, &rel16),
            Err(PerimeterRefusal::Footprint {
                side: "semaphore",
                need: 16,
                have: 4
            })
        );
        assert_eq!(
            put_host_sem_execute(&mut out, 0, at(0x1000, 8), (1, 0), &ts, false),
            Err(PerimeterRefusal::Footprint {
                side: "semaphore",
                need: 16,
                have: 8
            })
        );
        assert_eq!(
            put_host_sem_execute(&mut out, 0, at(0x1000, 4), (1, 0), &p64, false),
            Err(PerimeterRefusal::Footprint {
                side: "semaphore",
                need: 8,
                have: 4
            })
        );
        assert!(out.is_empty());
        assert!(put_host_semaphore(&mut out, 0, at(0x1000, 16), 1, &rel16).is_ok());
        assert!(put_host_semaphore(&mut out, 0, at(0x1000, 4), 1, &acq).is_ok());
        assert!(put_host_sem_execute(&mut out, 0, at(0x1000, 16), (1, 0), &ts, false).is_ok());
    }

    fn piece(launch: u32, regs: PieceRegs) -> LaunchPiece {
        LaunchPiece {
            sub: 4,
            tier: Tier::C8b5,
            launch: crate::ttables::decode_launch(Tier::C8b5, launch).unwrap(),
            regs,
            src: None,
            dst: None,
            sema: None,
        }
    }

    /// ★★ Gate-3 row `put_launch_piece` (review fix 2026-10-04): the footprint is recomputed from
    /// the registers emitted — remap `NUM_DST_COMPONENTS`, `PITCH × (LINE_COUNT − 1)`, a 16-byte
    /// four-word release — and a piece whose operands were validated for fewer bytes is refused
    /// with nothing emitted; operands must match what the launch touches.
    #[test]
    fn a_launch_piece_footprint_must_fit_its_validated_bytes() {
        // A remap fill, 4-byte components x2 (UVM's memset_8): 0x10 elements = 0x80 bytes.
        let fill = 2 | (1 << 7) | (1 << 8) | (1 << 10); // NON_PIPELINED, both PITCH, REMAP
        let remap8 = 4 | (5 << 4) | (3 << 16) | (1 << 24);
        let mut p = piece(
            fill,
            PieceRegs {
                line_len: 0x10,
                remap: remap8,
                ..PieceRegs::default()
            },
        );
        let mut out = Vec::new();
        p.dst = Some(at(0x10_0000, 0x40)); // validated for one component only
        assert_eq!(
            put_launch_piece(&mut out, &p),
            Err(PerimeterRefusal::Footprint {
                side: "dst",
                need: 0x80,
                have: 0x40
            })
        );
        assert!(out.is_empty());
        p.dst = Some(at(0x10_0000, 0x80));
        assert!(put_launch_piece(&mut out, &p).is_ok());
        // Multi-line: PITCH_OUT x (LINE_COUNT-1) + LINE_LENGTH.
        let ml = 2 | (1 << 7) | (1 << 8) | (1 << 9); // a multi-line copy
        let mut p = piece(
            ml,
            PieceRegs {
                line_len: 0x100,
                line_count: 4,
                pitch_in: 0x100,
                pitch_out: 0x1000,
                ..PieceRegs::default()
            },
        );
        p.src = Some(at(0x10_0000, 0x400));
        p.dst = Some(at(0x20_0000, 0x400)); // what LINE_LENGTH x LINE_COUNT would claim
        assert_eq!(
            put_launch_piece(&mut out, &p),
            Err(PerimeterRefusal::Footprint {
                side: "dst",
                need: 0x3100,
                have: 0x400
            })
        );
        p.dst = Some(at(0x20_0000, 0x3100));
        assert!(put_launch_piece(&mut out, &p).is_ok());
        // A four-word release needs 16 bytes; a two-word payload 8.
        let mut p = piece(1 << 4, PieceRegs::default()); // SEMAPHORE_TYPE 2, no data
        p.sema = Some(PieceSema {
            at: at(0x30_0000, 4),
            payload: 1,
            upper: None,
        });
        assert!(matches!(
            put_launch_piece(&mut out, &p),
            Err(PerimeterRefusal::Footprint { need: 16, .. })
        ));
        let mut p = piece((1 << 3) | (1 << 27), PieceRegs::default());
        p.sema = Some(PieceSema {
            at: at(0x30_0000, 4),
            payload: 1,
            upper: Some(2),
        });
        assert!(matches!(
            put_launch_piece(&mut out, &p),
            Err(PerimeterRefusal::Footprint { need: 8, .. })
        ));
        // Operands must match what the launch touches; unnamed remap bits are refused.
        let mut p = piece(fill, PieceRegs::default());
        assert!(matches!(
            put_launch_piece(&mut out, &p),
            Err(PerimeterRefusal::Operands { .. })
        ));
        p.dst = Some(at(0x10_0000, 0x1000));
        p.regs.remap = 4 | (1 << 31);
        assert!(matches!(
            put_launch_piece(&mut out, &p),
            Err(PerimeterRefusal::Field { .. })
        ));
    }

    #[test]
    fn the_ring_gp_entry_stays_in_the_pushbuffer() {
        assert!(ring_gp_entry(0xFF_0000_0000, 0xD_0000, 0, 0x100).is_some());
        assert!(ring_gp_entry(0xFF_0000_0000, 0xD_0000, 0xC_FF00, 0x200).is_none());
        assert!(ring_gp_entry(VA_LIMIT_40, 0xD_0000, 0, 0x100).is_none());
    }
}
