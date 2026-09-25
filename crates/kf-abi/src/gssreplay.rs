//! ★★ **GSS-legacy queries answered by the HOST, asked at realize with requests WE author.**
//!
//! ## The wall this module removes
//!
//! `[measured 2026-09-26, vvid, GA106 / 580.159.04, nvdiff LD_PRELOAD recorder on the SAME
//! static ffmpeg on bare metal and in a kayfabe guest]` `ffmpeg -c:v h264_nvenc` failed in the
//! guest with *"OpenEncodeSessionEx failed: unsupported device (2)"*. The ioctl diff named the
//! divergence: `libnvidia-encode` asks two GSS-legacy controls (bit 15 set: no open header names
//! them; the guest's CPU-RM routes them to the GSP) right after it builds its CUDA context —
//!
//! - `0x20809064` (520 B): request `{0, 1, 8}` → host `{0, 1, 8, 0x00100000, 1, 0x64}` — one entry
//!   naming clock domain `0x00100000` (the NVD / video clock). The guest was served the cudart
//!   constant from `cudartinit` (`{0, 2, 1, …}`: two entries, GPC and MCLK — captured for a
//!   DIFFERENT request), so the library went on to ask about the WRONG domain;
//! - `0x2080a028` (0x890 B): the clock query for that domain → refused `0x56` in the guest; the
//!   host answers it, and without it the library declares the device unsupported.
//!
//! ## What was measured about their layouts (host probe, `gssprobe`, 2026-09-26)
//!
//! `0x2080a028`: `{u32 1, u32 out, u32 count, entry[32] of 16 B {domain, 0, kHz, 0} at 0x0c, a
//! second per-domain array at 0x20c that must NAME THE SAME DOMAIN}`. With the 0x20c word zero the
//! host refuses `0x1f` (the bare-metal request carried it — what first read as uninitialised tail
//! was not all tail). The host writes `out = 4`, the entry's kHz (`0x0f55c8` = 1005 MHz NVD) and
//! `{2, min, max}` after 0x20c. A two-domain request with one mirror word is refused — so only
//! single-domain requests are stated here.
//!
//! ## The rule
//!
//! ⊘ **Never forwarded.** Each [`Row`] names a control, its size and the INPUT words we
//! understand. At realize the device asks the host exactly that request (zero elsewhere) and keeps
//! the reply. A guest request is answered only if its size and every named input word match a row
//! — then the bytes the host CHANGED are written into the guest's buffer and nothing else (the
//! guest's own uninitialised bytes survive, as they do on bare metal). Everything else falls
//! through to whatever answered before (for `0x20809064`, `cudartinit`'s measured cudart row).
//! The answers are the host's at VM start: clocks move; a statement of capability does not.

/// `0x20809064` — the GSS-legacy clock-domain listing.
pub const GSS_CLOCK_DOMAINS: u32 = 0x2080_9064;
/// `0x2080a028` — the GSS-legacy per-domain clock query.
pub const GSS_CLOCK_QUERY: u32 = 0x2080_a028;
/// The NVD (video engine) clock domain.
pub const DOMAIN_NVD: u32 = 0x0010_0000;
/// The GPC clock domain.
pub const DOMAIN_GPC: u32 = 0x0000_0001;

/// One authored request: `(cmd, paramsSize, [(byte offset, u32 value)])`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The control.
    pub cmd: u32,
    /// Its measured `paramsSize`.
    pub size: usize,
    /// The input words that select the answer (everything else is zero in the request).
    pub inputs: &'static [(usize, u32)],
}

/// ★ Every request this device asks its host at realize.
pub const ROWS: &[Row] = &[
    // The encoder's listing request `{0, 1, 8}` (measured, bare-metal libnvidia-encode).
    Row { cmd: GSS_CLOCK_DOMAINS, size: 520, inputs: &[(0, 0), (4, 1), (8, 8)] },
    // One-domain clock queries for the two domains the library has been seen to ask.
    Row { cmd: GSS_CLOCK_QUERY, size: 0x890, inputs: &[(0, 1), (8, 1), (0x0c, DOMAIN_NVD), (0x20c, DOMAIN_NVD)] },
    Row { cmd: GSS_CLOCK_QUERY, size: 0x890, inputs: &[(0, 1), (8, 1), (0x0c, DOMAIN_GPC), (0x20c, DOMAIN_GPC)] },
];

impl Row {
    /// The request buffer: zero, with the input words set.
    #[must_use]
    pub fn request(&self) -> Vec<u8> {
        let mut p = vec![0u8; self.size];
        for &(o, v) in self.inputs {
            if let Some(w) = p.get_mut(o..o + 4) {
                w.copy_from_slice(&v.to_le_bytes());
            }
        }
        p
    }

    /// Does the guest's `params` ask this row's question?
    #[must_use]
    pub fn matches(&self, cmd: u32, params: &[u8]) -> bool {
        cmd == self.cmd
            && params.len() == self.size
            && self.inputs.iter().all(|&(o, v)| params.get(o..o + 4).is_some_and(|w| w == v.to_le_bytes()))
    }
}

/// The host's answer to one [`Row`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The row asked.
    pub row: Row,
    /// `(offset, byte)` for every byte the host changed.
    pub wrote: Vec<(usize, u8)>,
}

impl Answer {
    /// Record the host's reply to `row.request()`.
    #[must_use]
    pub fn from_host(row: Row, reply: &[u8]) -> Answer {
        let req = row.request();
        let wrote = req.iter().zip(reply).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (_, b))| (i, *b)).collect();
        Answer { row, wrote }
    }
}

/// ★ Answer a guest request from the host's realize-time answers; `false` = not ours.
#[must_use]
pub fn answer(answers: &[Answer], cmd: u32, params: &mut [u8]) -> bool {
    let Some(a) = answers.iter().find(|a| a.row.matches(cmd, params)) else { return false };
    for &(i, b) in &a.wrote {
        params[i] = b;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured host pair for the NVD query, reproduced through the rule.
    #[test]
    fn the_nvd_clock_query_is_answered_with_exactly_what_the_host_wrote() {
        let row = ROWS[1];
        let mut host = row.request();
        host[4] = 4;
        host[0x14..0x18].copy_from_slice(&0x000f_55c8u32.to_le_bytes());
        host[0x210] = 2;
        let a = Answer::from_host(row, &host);
        assert_eq!(a.wrote.len(), 1 + 3 + 1);
        // The guest's request as bare-metal libnvidia-encode builds it: uninitialised elsewhere.
        let mut g = vec![0x5au8; 0x890];
        for &(o, v) in row.inputs {
            g[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        g[4..8].fill(0);
        g[0x14..0x18].fill(0);
        assert!(answer(&[a.clone()], GSS_CLOCK_QUERY, &mut g));
        assert_eq!(&g[4..8], &4u32.to_le_bytes());
        assert_eq!(&g[0x14..0x18], &0x000f_55c8u32.to_le_bytes());
        assert_eq!(g[0x210], 2);
        assert_eq!(g[0x30], 0x5a, "a byte the host did not write is the guest's");
        // ⊘ Another domain, another size, another control: not ours.
        let mut other = row.request();
        other[0x0c] = 7;
        assert!(!answer(&[a.clone()], GSS_CLOCK_QUERY, &mut other));
        assert!(!answer(&[a.clone()], GSS_CLOCK_QUERY, &mut vec![0; 16]));
        assert!(!answer(&[a], GSS_CLOCK_DOMAINS, &mut row.request()));
    }

    #[test]
    fn the_listing_request_is_the_encoders() {
        let r = ROWS[0].request();
        assert_eq!(&r[..12], &[0, 0, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0]);
        let mut guest = r.clone();
        guest[24..32].copy_from_slice(&0x7dea_9802_b650u64.to_le_bytes()); // a pointer, measured
        assert!(ROWS[0].matches(GSS_CLOCK_DOMAINS, &guest));
        let mut cudart = r;
        cudart[8] = 0;
        assert!(!ROWS[0].matches(GSS_CLOCK_DOMAINS, &cudart), "cudart's request stays cudartinit's");
    }
}
