//! The user register access map we **serve** to the guest — `GET_USER_REGISTER_ACCESS_MAP`
//! (`0x20800a41`).
//!
//! ## ★★★ We author this. It is not something to discover.
//!
//! `[owner, 2026-09-21]` *"but can we then construct fake data so that ogkm satisfies"* — yes, and
//! ogkm's own source says exactly what it will accept, so none of this is guesswork:
//!
//! | ogkm fact | `src/kernel/gpu/gpu_register_access_map.c` |
//! |---|---|
//! | the map is a **bitfield, one bit per 32-bit register** | `bitOffset = offset / sizeof(NvU32)`, `:141` |
//! | a userspace access is gated by one bit | `nvBitFieldTest(...)`, `:171` |
//! | ⊘ **`compressedSize == 0` ⇒ the driver fills the map with `0xFF` itself** | `:~262` |
//!
//! ⇒ **The cheapest satisfying answer is a declared ABSENCE, not a forged presence**: return
//! `compressedSize = 0` and the guest never decompresses anything.
//!
//! ## ⊘⊘⊘ AND THAT EASY ANSWER IS THE WRONG ONE, which is the point
//!
//! `0xFF` everywhere tells the guest that **every BAR0 register is userspace-accessible**. §47
//! says a page unprivileged guest userspace can map must **not** carry a write trap — so if we
//! declare everything mappable, we may trap **nothing**, and the privileged arm of the classifier
//! ceases to exist. ⇒ The lazy fallback hands an unprivileged guest process the whole register
//! file and takes our own boundary away in the same move.
//!
//! ★ So we emit a **restrictive** map, built from the same classifier the trap path uses. That is
//! the declaration half of the invariant pinned in `tests/campaign_findings.rs`: **the map we
//! serve and `Class::UserspaceMappable` are one fact.**
//!
//! ## ⊘ How we produce a zlib stream with no compression library
//!
//! The guest inflates `compressedData`. A DEFLATE stream may carry **stored (uncompressed)
//! blocks** (BTYPE=00), so a valid stream is a 2-byte zlib header, a sequence of
//! `[final, LEN, ~LEN, raw bytes]` blocks, and an Adler-32 trailer. ⇒ **No deflate encoder, no
//! dependency, and nothing to get subtly wrong** — the bytes we emit are the bytes the guest gets.

/// One bit per 32-bit register. BAR0 is 16 MiB ⇒ 4 Mi registers ⇒ 512 KiB of bitmap.
pub const BAR0_BYTES: u32 = 16 << 20;
pub const MAP_BITS: usize = (BAR0_BYTES / 4) as usize;
pub const MAP_BYTES: usize = MAP_BITS / 8;

/// The map, as the guest will test it.
pub struct AccessMap {
    bits: Vec<u8>,
}

impl Default for AccessMap {
    fn default() -> Self {
        Self::deny_all()
    }
}

impl AccessMap {
    /// ⊘ **Deny by default**, so a register becomes userspace-reachable only by being named —
    /// the same posture as the class allowlist and the control allowlist.
    pub fn deny_all() -> AccessMap {
        AccessMap { bits: vec![0u8; MAP_BYTES] }
    }

    /// Allow one byte-range of BAR0. ⚠ `offset` and `len` are BYTE addresses, converted to
    /// register indices exactly as ogkm does (`offset / sizeof(NvU32)`).
    pub fn allow_range(&mut self, offset: u32, len: u32) {
        let first = (offset / 4) as usize;
        let last = ((offset + len).min(BAR0_BYTES) / 4) as usize;
        for reg in first..last {
            if let Some(b) = self.bits.get_mut(reg / 8) {
                *b |= 1 << (reg % 8);
            }
        }
    }

    /// ★ The guest's own test, so our side and its side cannot disagree about bit order.
    pub fn is_allowed(&self, offset: u32) -> bool {
        let reg = (offset / 4) as usize;
        self.bits.get(reg / 8).is_some_and(|b| b & (1 << (reg % 8)) != 0)
    }

    #[inline]
    pub fn raw(&self) -> &[u8] {
        &self.bits
    }

    /// Wrap the map in a zlib stream the guest's inflate will accept.
    ///
    /// ⊘ Stored blocks only. §RFC1950/1951: `0x78 0x01` (deflate, 32K window, no preset dict,
    /// FCHECK making the pair a multiple of 31), then `[BFINAL|BTYPE=00, LEN, ~LEN, data]`, then
    /// Adler-32 big-endian.
    pub fn to_zlib_stored(&self) -> Vec<u8> {
        let mut out = vec![0x78u8, 0x01];
        let data = &self.bits;
        let mut i = 0usize;
        if data.is_empty() {
            out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
        }
        while i < data.len() {
            let n = (data.len() - i).min(0xFFFF);
            let final_block = u8::from(i + n >= data.len());
            out.push(final_block); // BTYPE=00 (stored) in bits 1..2
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out.extend_from_slice(&(!(n as u16)).to_le_bytes());
            out.extend_from_slice(&data[i..i + n]);
            i += n;
        }
        out.extend_from_slice(&adler32(data).to_be_bytes());
        out
    }
}

/// Adler-32, as RFC 1950 requires in the trailer.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
