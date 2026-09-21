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
        // ⊘ `[fable]` checked add: `offset + len` wrapped before `.min()`, silently allowing nothing.
        let end = offset.saturating_add(len).min(BAR0_BYTES);
        let first = (offset / 4) as usize;
        let last = (end / 4) as usize;
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

    /// Wrap the map in a stream **ogkm will actually accept**.
    ///
    /// ## ⊘⊘⊘ THE FIRST VERSION WAS BOOT-FATAL, AND MY OWN TEST APPROVED IT
    ///
    /// `[fable w823, CRITICAL]` ogkm does **not** inflate a zlib stream. It skips a **10-byte
    /// gzip header** and raw-inflates the deflate payload:
    ///
    /// ```text
    /// pComprData += 10;                                  // gpu_register_access_map.c:362
    /// inflatedBytes = utilGzGetData(pComprData, ...);    // :364
    /// if (inflatedBytes != accessMapSize) return NV_ERR_INFLATE_COMPRESSED_DATA_FAILED;
    /// ```
    ///
    /// and `gpuConstructUserRegisterAccessMap` is called under `NV_ASSERT_OK_OR_RETURN`
    /// (`gpu.c:2183`) ⇒ **a bad stream aborts GPU init for every guest, on every die.**
    ///
    /// ⚠ I emitted zlib (`0x78 0x01` + Adler-32) and *verified it with Python's `zlib.decompress`*
    /// — which accepts zlib. **I tested against a different decoder than the consumer uses**, and
    /// a green result on the wrong oracle is worth less than no test.
    ///
    /// ⊘ **And it could never have fitted anyway.** The reply struct caps the payload at
    /// **16 384** bytes (610, `ctrl2080internal.h`) and **4 096** on 580 — a stored-block encoding
    /// of a 512 KiB map is **524 339** bytes, 128× over. The real GA106 reply is ~1.3 KB, because
    /// the map is near-uniform and compresses hard. ⇒ *"No deflate encoder, nothing to get subtly
    /// wrong"* was exactly backwards: skipping the encoder is what made it impossible.
    ///
    /// ★ So this emits a gzip container over **fixed-Huffman deflate**, which a near-uniform
    /// bitmap compresses far below the cap.
    pub fn to_gzip_deflate(&self) -> Vec<u8> {
        // RFC 1952 header: magic, CM=deflate, no flags, no mtime, XFL=0, OS=unknown.
        // ⊘ ogkm skips exactly 10 bytes, so the header must be exactly 10 — no FNAME, no FEXTRA.
        let mut out = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xff];
        out.extend_from_slice(&deflate_fixed_rle(&self.bits));
        out
    }

    /// ⚠ The payload cap the reply struct imposes. §50: a fact from compilable ogkm C.
    pub const MAX_COMPRESSED_610: usize = 16384;
    pub const MAX_COMPRESSED_580: usize = 4096;
}

/// Fixed-Huffman DEFLATE with run-length matching — enough for a near-uniform bitmap, and small
/// enough to read in one sitting. ⊘ Not a general compressor: it emits literals and back-references
/// only, which is all a map of mostly-identical bytes needs.
fn deflate_fixed_rle(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bits(1, 1); // BFINAL
    w.bits(1, 2); // BTYPE = 01, fixed Huffman
    let mut i = 0usize;
    while i < data.len() {
        // Find a run of the same byte; a repeat of length >= 3 becomes a back-reference of
        // distance 1, which is how RLE is spelled in DEFLATE.
        // ⊘ Scan the WHOLE run. Capping the scan at 258 (the max match length) re-emitted a
        // literal every 258 bytes and cost ~2x: 6 620 bytes for a map that fits in 2 371.
        // The 258 limit belongs on each back-reference, not on the run.
        let mut run = 1usize;
        while i + run < data.len() && data[i + run] == data[i] {
            run += 1;
        }
        lit(&mut w, data[i]);
        let mut left = run - 1;
        while left >= 3 {
            let n = left.min(258);
            length_dist1(&mut w, n);
            left -= n;
        }
        for _ in 0..left {
            lit(&mut w, data[i]);
        }
        i += run;
    }
    lit_code(&mut w, 256); // end of block
    w.finish()
}

#[derive(Default)]
struct BitWriter { out: Vec<u8>, acc: u32, n: u32 }
impl BitWriter {
    /// DEFLATE packs Huffman codes MSB-first but other fields LSB-first; `bits` is the LSB-first
    /// form used for BFINAL/BTYPE and the extra bits.
    fn bits(&mut self, v: u32, n: u32) {
        self.acc |= v << self.n;
        self.n += n;
        while self.n >= 8 { self.out.push((self.acc & 0xff) as u8); self.acc >>= 8; self.n -= 8; }
    }
    /// Huffman codes are written most-significant-bit first.
    fn code(&mut self, v: u32, n: u32) {
        for k in (0..n).rev() { self.bits((v >> k) & 1, 1); }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 { self.out.push((self.acc & 0xff) as u8); }
        self.out
    }
}

/// RFC 1951 §3.2.6 fixed literal/length code.
fn lit_code(w: &mut BitWriter, sym: u32) {
    match sym {
        0..=143 => w.code(0x30 + sym, 8),
        144..=255 => w.code(0x190 + (sym - 144), 9),
        256..=279 => w.code(sym - 256, 7),
        _ => w.code(0xc0 + (sym - 280), 8),
    }
}
fn lit(w: &mut BitWriter, b: u8) { lit_code(w, b as u32); }

/// Emit a length `n` (3..=258) with distance 1.
fn length_dist1(w: &mut BitWriter, n: usize) {
    // RFC 1951 §3.2.5 length codes.
    let (code, extra_bits, base) = match n {
        3..=10 => (257 + (n - 3) as u32, 0u32, n),
        11..=18 => (265 + ((n - 11) / 2) as u32, 1, 11 + ((n - 11) / 2) * 2),
        19..=34 => (269 + ((n - 19) / 4) as u32, 2, 19 + ((n - 19) / 4) * 4),
        35..=66 => (273 + ((n - 35) / 8) as u32, 3, 35 + ((n - 35) / 8) * 8),
        67..=130 => (277 + ((n - 67) / 16) as u32, 4, 67 + ((n - 67) / 16) * 16),
        131..=257 => (281 + ((n - 131) / 32) as u32, 5, 131 + ((n - 131) / 32) * 32),
        _ => (285, 0, 258),
    };
    lit_code(w, code);
    if extra_bits > 0 { w.bits((n - base) as u32, extra_bits); }
    w.code(0, 5); // distance code 0 => distance 1, no extra bits
}


