// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ Census keys made from what a site SAYS, never from where it lives (2026-10-04,
//! `v3-sec-rawaddr`, audit S1-02).
//!
//! The lock and trap census tables used to key their slots by the ADDRESS of a `&'static str`
//! or of a `core::panic::Location` (`as_ptr() as usize`, `ptr::from_ref(..) as usize`). That
//! was benign — the number was never dereferenced or printed — but it is a host address held as
//! an integer in safe code, which is exactly the shape the pointer gates cannot tell from a real
//! one. A key built from the text and the line is an ordinary number.
//!
//! ⊘ **What it costs, said plainly.** Two different texts can hash to one key; the census then
//! counts both under the FIRST text that claimed the slot. A census that merges two rows is
//! wrong as a statistic and harmless as anything else — these tables never gate behaviour.

/// The 16 bytes at the END of `b` (fewer, zero-padded at the front, when `b` is shorter) as two
/// little-endian words: two loads, no per-byte loop.
fn tail_words(b: &[u8]) -> (u64, u64) {
    let t: [u8; 16] = if let Some(t) = b.last_chunk::<16>() {
        *t
    } else {
        let mut w = [0u8; 16];
        w[16 - b.len()..].copy_from_slice(b);
        w
    };
    let (lo, hi) = t.split_at(8);
    (
        u64::from_le_bytes(lo.try_into().unwrap_or_default()),
        u64::from_le_bytes(hi.try_into().unwrap_or_default()),
    )
}

/// A 64-bit multiply mix.
fn mix(x: u64) -> u64 {
    (x ^ (x >> 32)).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// How many leading and trailing bytes of a text the keys read. ⊘ Bounded on purpose (review of
/// `v3-sec-rawaddr`, 2026-10-04): the keys are computed on EVERY ranked lock acquisition and every
/// inline mint, including inside MMIO traps, so they read a fixed number of words, never the whole
/// text byte by byte (the first version ran FNV-1a over all of a reason's bytes).
pub(crate) const SITE_TAIL_BYTES: usize = 16;

/// The census key of a `&'static str` reason: its length, its first and its last
/// [`SITE_TAIL_BYTES`] bytes, mixed. Never `0` (the tables use `0` for "free"). O(1).
///
/// ⊘ Two reasons collide only when they share their length and both 16-byte ends — then their
/// census rows merge (module docs).
#[must_use]
pub(crate) fn str_key(text: &str) -> usize {
    let b = text.as_bytes();
    let (h0, h1) = tail_words(&b[..b.len().min(SITE_TAIL_BYTES)]);
    let (t0, t1) = tail_words(b);
    let k = mix(h0 ^ h1.rotate_left(17) ^ t0.rotate_left(31) ^ t1.rotate_left(47) ^ b.len() as u64);
    // Truncation to a 32-bit `usize` keeps the low half, which is as well mixed as the rest.
    (k as usize) | 1
}

/// The census key of a call site: `(line << 40) ^ (column << 24) ^ (file length << 8)` mixed
/// with an 8-bit digest of the file name's last [`SITE_TAIL_BYTES`] bytes (two word loads). Never
/// `0`. O(1).
///
/// ⊘ Two sites collide only when they share line, column, file-name length and that 8-bit tail
/// digest — then their census rows merge (module docs).
#[must_use]
pub(crate) fn site_key(file: &str, line: u32, column: u32) -> usize {
    let b = file.as_bytes();
    let (lo, hi) = tail_words(b);
    let tail8 = mix(lo ^ hi.rotate_left(29)) >> 56;
    let key = (u64::from(line) << 40)
        ^ (u64::from(column) << 24)
        ^ ((b.len() as u64 & 0xffff) << 8)
        ^ tail8;
    // Fold the high half into the low one for a 32-bit `usize`; a no-op on 64-bit.
    let key = if usize::BITS < 64 {
        key ^ (key >> 32)
    } else {
        key
    };
    (key as usize) | 1
}

/// A probe start for an open-addressed table: the key's bits fully mixed (the `splitmix64`
/// finaliser), so keys that differ in any field land apart. O(1).
#[must_use]
pub(crate) fn probe(key: usize) -> usize {
    let mut z = key as u64;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T24: distinct sites get distinct keys; one site always gets the same key; equal texts
    /// share a key and different ones (here) do not.
    #[test]
    fn keys_follow_the_text_and_the_line_not_an_address() {
        let a = site_key("crates/kf-qemu/src/chan.rs", 120, 9);
        assert_eq!(a, site_key("crates/kf-qemu/src/chan.rs", 120, 9));
        assert_ne!(
            a,
            site_key("crates/kf-qemu/src/chan.rs", 121, 9),
            "another line"
        );
        assert_ne!(
            a,
            site_key("crates/kf-qemu/src/chan.rs", 120, 13),
            "another column"
        );
        assert_ne!(
            a,
            site_key("crates/kf-qemu/src/mem.rs", 120, 9),
            "another file"
        );
        assert_ne!(
            a,
            site_key("crates/kf-qemu/src/xhan.rs", 120, 9),
            "same length, other tail"
        );
        assert_ne!(a & 1, 0);
        assert_ne!(site_key("", 0, 0), 0, "never the free marker");
        let s = String::from("cuCtxSynchronize on a vCPU");
        assert_eq!(
            str_key("cuCtxSynchronize on a vCPU"),
            str_key(&s),
            "a key is the text, wherever the bytes live"
        );
        assert_ne!(str_key("a"), str_key("b"));
        assert_ne!(str_key(""), 0);
    }

    /// The probe start spreads the sites of ONE file over the table: 64 consecutive lines must
    /// not share a handful of starts (the old address mixing ignored the line entirely).
    #[test]
    fn one_files_sites_spread_over_the_table() {
        let starts: std::collections::BTreeSet<usize> = (1..=64)
            .map(|line| probe(site_key("crates/kf-util/src/lock.rs", line, 21)) % 1024)
            .collect();
        assert!(starts.len() > 48, "{} distinct starts of 64", starts.len());
    }

    /// The key reads at most the last [`SITE_TAIL_BYTES`] bytes of the file name: two names that
    /// differ only before that tail (same length) collide BY DESIGN — this is the documented
    /// O(1) bound, pinned so a change to it is a decision.
    #[test]
    fn the_site_key_reads_only_the_tail_of_the_file_name() {
        let tail = "/src/abc/lock.rs";
        assert_eq!(tail.len(), SITE_TAIL_BYTES);
        let x = format!("crates/kf-aaaa{tail}");
        let y = format!("crates/kf-bbbb{tail}");
        assert_eq!(site_key(&x, 7, 3), site_key(&y, 7, 3));
        // ...and a long name costs no more than a short one: no panic on a non-ASCII boundary.
        let z = format!("{}é{tail}", "x".repeat(1000));
        let _ = site_key(&z, 1, 1);
        let _ = site_key("é", 1, 1);
    }

    /// ★ The reason key is O(1) too (review of `v3-sec-rawaddr`, 2026-10-04): it reads the length
    /// and both 16-byte ends, so two texts of one length that differ only in the middle collide BY
    /// DESIGN, while a different end or length does not. A key over every byte would make the
    /// first pair differ — and cost the trap path a loop over the whole text.
    #[test]
    fn the_reason_key_reads_a_bounded_number_of_bytes() {
        let head = "cuCtxSynchronize";
        let tail = "on a vCPU thread";
        let a = format!("{head} AAAA {tail}");
        let b = format!("{head} BBBB {tail}");
        assert_eq!(str_key(&a), str_key(&b), "only the middle differs");
        assert_ne!(
            str_key(&a),
            str_key(&format!("{head} AAAA {tail}.")),
            "the end"
        );
        assert_ne!(str_key(&a), str_key(&format!("X{}", &a[1..])), "the start");
        assert_ne!(
            str_key(&a),
            str_key(&format!("{head} AAAAA {tail}")),
            "the length"
        );
        assert_ne!(str_key("x"), str_key("y"), "a short text, wholly read");
        assert_ne!(str_key("é"), 0);
    }
}
