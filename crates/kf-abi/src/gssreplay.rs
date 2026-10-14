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
//! ## ★★★ v3-refusals (2026-09-26): cudart's clock query, and the byte a zero probe cannot see
//!
//! `[measured vrf, GA102 / 580.159.04, nvdiff host vs kf3 guest, 22 workloads]` libcudart asks
//! `0x2080a084` (4 B) then `0x2080a026` (532 B) in every process — 22 times in `torch_correct`, 54 in
//! `llama.cpp` — and kf3 refused both `0x56`. The guest's cudart then took the `0x2080a001` FALLBACK
//! (`crate::cudartinit`) and reported **`GPU Max Clock rate: 420 MHz`** where the host's
//! `deviceQuery` says **1695 MHz**: every `cudaDevAttrClockRate` in the guest was wrong. The host
//! answers `0x2080a026` `{0x400, …, count 2, {GPC 1, 0, kHz, 0}, {MCLK 0x10, 0, kHz, 0}}` with the
//! max clocks (`1695000`, `9751000` kHz — identical over 135 calls) and writes a `u8` at 4 and
//! `u32`s at 8 and 0x0c that the guest's request carries as UNINITIALISED stack bytes.
//!
//! ⊘ A single zero-filled probe cannot tell *"the host wrote 0"* from *"the host did not touch it"*,
//! so a field the host overwrites with `1` would leave three bytes of the guest's garbage above it
//! (`0xwwzzyy01`) where bare metal returns `1`. [`Answer::from_probes`] therefore asks each row twice
//! — over a zero background and over [`PROBE_BACKGROUND`] — and a byte is the host's if it changed in
//! EITHER; its value must be the same in both, or the background is taken to have changed the answer
//! and only the zero probe's difference is kept (the pre-v3-refusals behaviour, never wider).
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

/// The MCLK (memory) clock domain.
pub const DOMAIN_MCLK: u32 = 0x0000_0010;

/// ★ v3-refusals: `0x2080a084` — cudart's 4-byte precursor to [`GSS_CUDART_CLOCKS`]; request `0`,
/// host `NV_OK`, writes nothing (136 of 136 bare-metal calls).
pub const GSS_CUDART_A084: u32 = 0x2080_a084;
/// ★ v3-refusals: `0x2080a026` — cudart's max-clock query for GPC and MCLK (the source of
/// `cudaDevAttrClockRate` / `MemoryClockRate`); 532 bytes.
pub const GSS_CUDART_CLOCKS: u32 = 0x2080_a026;
/// ★ v3-refusals: the second probe's background (see the module doc). Any value the requests
/// never carry in an input word works; `0xa5` is not `0`, not `0xff`, and not ASCII.
pub const PROBE_BACKGROUND: u8 = 0xa5;

/// ★ `0x20808163` — acquire one NVENC session slot (4 bytes, `0` in and out).
///
/// `[measured vvid 2026-09-26, host probe]` the 9th acquire on one GA106 returns `0x69`
/// (insufficient resources): it is the GPU-WIDE GeForce encoder-session cap, and `0x20808164`
/// releases a slot. In a guest it was refused `0x56` and `libnvidia-encode` failed
/// `OpenEncodeSessionEx` with *"incompatible client key (21)"*. ⊘ So it is NOT answerable from a
/// realize-time reply (it is state, not a fact): a guest acquire is an acquire of the HOST's slot
/// on our client (authored request, the channel plane's act), released with the guest's release
/// or its client's free — the host's cap then bounds guests and host processes alike.
pub const GSS_ENC_SESSION_ACQUIRE: u32 = 0x2080_8163;
/// `0x20808164` — release one NVENC session slot.
pub const GSS_ENC_SESSION_RELEASE: u32 = 0x2080_8164;
/// Both carry exactly 4 bytes, measured `0` in and out.
pub const ENC_SESSION_PARAMS_SIZE: usize = 4;

/// One authored request: `(cmd, paramsSize, [(byte offset, u32 value)])`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The control.
    pub cmd: u32,
    /// Its measured `paramsSize`.
    pub size: usize,
    /// The input words that select the answer (everything else is zero in the request).
    pub inputs: &'static [(usize, u32)],
    /// ★ Lane C 2026-10-14: the guest's request must equal [`Row::request`] byte for byte
    /// (every byte not named in `inputs` zero). For controls whose request layout is unknown but
    /// whose real-driver request was MEASURED all-zero: no word is guessed to be an input, and any
    /// non-zero byte makes it a different question.
    pub exact: bool,
}

/// ★ Every request this device asks its host at realize.
pub const ROWS: &[Row] = &[
    // The encoder's listing request `{0, 1, 8}` (measured, bare-metal libnvidia-encode).
    Row {
        cmd: GSS_CLOCK_DOMAINS,
        size: 520,
        exact: false,
        inputs: &[(0, 0), (4, 1), (8, 8)],
    },
    // One-domain clock queries for the two domains the library has been seen to ask.
    Row {
        cmd: GSS_CLOCK_QUERY,
        size: 0x890,
        exact: false,
        inputs: &[(0, 1), (8, 1), (0x0c, DOMAIN_NVD), (0x20c, DOMAIN_NVD)],
    },
    Row {
        cmd: GSS_CLOCK_QUERY,
        size: 0x890,
        exact: false,
        inputs: &[(0, 1), (8, 1), (0x0c, DOMAIN_GPC), (0x20c, DOMAIN_GPC)],
    },
    // ★ v3-refusals: cudart's pair, the request words exactly as recorded calls carried them — `a084`
    // 136 bare-metal calls; `a026` 270 (135 bare metal + 135 guest, one cudart binary). Every word
    // CONSTANT across them is named; the varying ones (0x04, 0x0c, 0x34..) are the caller's
    // uninitialised stack.
    Row {
        cmd: GSS_CUDART_A084,
        size: 4,
        exact: false,
        inputs: &[(0, 0)],
    },
    Row {
        cmd: GSS_CUDART_CLOCKS,
        size: 532,
        exact: false,
        inputs: &[
            (0x00, 0x400),
            (0x08, 0),
            (0x10, 2),
            (0x14, DOMAIN_GPC),
            (0x18, 0),
            (0x1c, 0),
            (0x20, 0),
            (0x24, DOMAIN_MCLK),
            (0x28, 0),
            (0x2c, 0),
            (0x30, 0),
        ],
    },
    // ★★ Lane C 2026-10-14 — cudart's init gate asked of the HOST (tier 2) instead of served from
    // one GA106 capture to every die (`crate::cudartinit` rows → `kf_rm::consttab`, now the
    // fallback). `[measured AD104 / 595.91.07, 2026-10-14, unprivileged libcudart 13.3 under the
    // nvdiff recorder, traces/constants_night_20261014/]` cudart's own requests: `0x20809009` and
    // `0x20809001` all zero (8 B), `0x20809064` `{0, 2, 1, 0, 0, 0, 4, 0…}` (520 B) — every word
    // named here; the host answered each `NV_OK` with exactly the GA106 rows' values.
    Row {
        cmd: CUDART_GSS_9009,
        size: 8,
        exact: false,
        inputs: &[(0, 0), (4, 0)],
    },
    Row {
        cmd: CUDART_GSS_9001,
        size: 8,
        exact: false,
        inputs: &[(0, 0), (4, 0)],
    },
    Row {
        cmd: GSS_CLOCK_DOMAINS,
        size: 520,
        exact: false,
        inputs: &[(0, 0), (4, 2), (8, 1), (0x18, 4)],
    },
    // cudart's fallback, reached only when `a084`/`a026` fail; the guest's request is `{0,0,0,0}`
    // (`[measured GA106 / 580.159.04, 2026-08-20]`, `crate::cudartinit::CUDART_INIT_0XA001`).
    Row {
        cmd: CUDART_GSS_A001,
        size: 16,
        exact: false,
        inputs: &[(0, 0), (4, 0), (8, 0), (0x0c, 0)],
    },
    // ★★ Lane C 2026-10-14 — Windows-only GSS-legacy queries whose real-driver request is MEASURED
    // all-zero (`[measured AD103 / Windows 580.88, GSP firmware 580.65.05, kfwc capture, 2026-10-14]`
    // request bodies: 0x2080852a x61, 0x20808524 x3, 0x2080a70a x3, 0x2080852e x55, 0x2080853a x3,
    // 0x2080a618 x58, 0x2080a088 6 of 12 calls) and that the host answers NV_OK to a zero request
    // unprivileged (`[measured AD103 / Linux 575.51.03, traces/real_hw_20261013/probe_unpriv.txt]`
    // all seven; `[measured AD104 / 595.91.07, traces/ctl_probe_20261013/]` 852a, 8524, a70a, a088).
    // Purpose unknown (no open header): the guest gets the HOST's bytes for the identical question,
    // never a captured row; any non-zero request byte is a different question (refused as before).
    Row {
        cmd: 0x2080_852a,
        size: 1784,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_8524,
        size: 4,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_a70a,
        size: 26,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_a088,
        size: 12,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_852e,
        size: 528,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_853a,
        size: 1456,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_a618,
        size: 8620,
        exact: true,
        inputs: &[],
    },
    // ★★ Lane C 2026-10-14 (resumed, daytime) — the SAME controls at the 595 firmware branch. Their paramsSize changed with
    // the branch (the request stays all-zero): `[measured GA104 / Windows 596.51, GSP firmware 595.76, kfwc capture,
    // 2026-10-14, traces/constants_measure_20261014b/ga104_596.51/]` 0x2080852e 596 (was 528), 0x2080853a 15352 (was 1456),
    // 0x2080a618 8632 (was 8620), 0x2080853b 14320 with request word 1 = 1 (the AD103 580.88 capture issued it once at 1832);
    // 0x2080852a, 0x20808524, 0x2080a70a, 0x2080a088 are unchanged. A host answers a zero request only at the size of its own
    // branch (`[measured, traces/constants_measure_20261014b/tier2/]` 580.x hosts: 528/1456/8620 NV_OK and 596/15352/8632
    // INVALID_ARGUMENT; 595.84 / 595.91.07 hosts: the reverse) — that is F-4 of the matrix: the INVALID_ARGUMENT at 595.91.07 was
    // the 580-branch SIZE, not the control. The size is part of the question (`Row::matches`), so a row whose size the host
    // refuses is simply not answered, and a guest of either branch gets the host's bytes for ITS size or a refusal, never
    // another branch's row.
    Row {
        cmd: 0x2080_852e,
        size: 596,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_853a,
        size: 15352,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_a618,
        size: 8632,
        exact: true,
        inputs: &[],
    },
    Row {
        cmd: 0x2080_853b,
        size: 14320,
        exact: true,
        inputs: &[(4, 1)],
    },
    // ★ The 610 branch (`[measured, tier-2 kit on hosts 610.43.02 (RTX 5090 GB202) and 615.71.09 (L4 AD104),
    // traces/constants_measure_20261014b/tier2/]` the host's smallest accepted zero-request size for 0x2080852e is 1432; 0x2080853a stays 15352,
    // 0x2080a618 is accepted at no size 4..16384 — the control is gone there). ⚠ Only the HOST side is measured: no Windows
    // 610 capture exists, so that a 610 guest sends an all-zero request of 1432 B is [I]; if it sends anything else this row
    // simply does not match (the size and every byte are part of the question).
    Row {
        cmd: 0x2080_852e,
        size: 1432,
        exact: true,
        inputs: &[],
    },
    // ★ Lane C 2026-10-14: `0x20808162` (1 byte, request `00`): the GA106 at 580.159.04 answered
    // `01` (`crate::gsslegacy`), the AD104 at 595.91.07 answers `00` [measured, unprivileged,
    // traces/constants_night_20261014/probe_8162_595.txt] — a captured byte served to every die was
    // wrong there. Asked of the host at realize instead.
    Row {
        cmd: 0x2080_8162,
        size: 1,
        exact: true,
        inputs: &[],
    },
];

/// `0x20809009` — cudart init gate (GSS-legacy, 8 B).
pub const CUDART_GSS_9009: u32 = 0x2080_9009;
/// `0x20809001` — cudart init gate (GSS-legacy, 8 B).
pub const CUDART_GSS_9001: u32 = 0x2080_9001;
/// `0x2080a001` — cudart's clock fallback (GSS-legacy, 16 B).
pub const CUDART_GSS_A001: u32 = 0x2080_a001;

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

    /// The request over a `background` fill (the input words set) — the second probe.
    #[must_use]
    pub fn request_over(&self, background: u8) -> Vec<u8> {
        let mut p = vec![background; self.size];
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
            && self
                .inputs
                .iter()
                .all(|&(o, v)| params.get(o..o + 4).is_some_and(|w| w == v.to_le_bytes()))
            && (!self.exact || params == self.request().as_slice())
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
        let wrote = req
            .iter()
            .zip(reply)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (_, b))| (i, *b))
            .collect();
        Answer { row, wrote }
    }

    /// ★ v3-refusals: record the host's replies to `row.request()` (`zero`) and to
    /// `row.request_over(PROBE_BACKGROUND)` (`bg`). A byte is the host's if it changed in either
    /// probe, and then it must carry the same value in both; if any does not, the background
    /// changed the answer and only [`Answer::from_host`]'s zero-probe difference is kept.
    /// Returns the answer and whether the second probe was usable.
    #[must_use]
    pub fn from_probes(row: Row, zero: &[u8], bg: Option<&[u8]>) -> (Answer, bool) {
        let narrow = Answer::from_host(row, zero);
        let Some(bg) = bg else { return (narrow, false) };
        let (rz, rb) = (row.request(), row.request_over(PROBE_BACKGROUND));
        if zero.len() != row.size || bg.len() != row.size {
            return (narrow, false);
        }
        let mut wrote = Vec::new();
        for i in 0..row.size {
            if zero[i] != rz[i] || bg[i] != rb[i] {
                if zero[i] != bg[i] {
                    return (narrow, false);
                }
                wrote.push((i, zero[i]));
            }
        }
        (Answer { row, wrote }, true)
    }
}

/// ★ Answer a guest request from the host's realize-time answers; `false` = not ours.
#[must_use]
pub fn answer(answers: &[Answer], cmd: u32, params: &mut [u8]) -> bool {
    let Some(a) = answers.iter().find(|a| a.row.matches(cmd, params)) else {
        return false;
    };
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
        assert!(answer(std::slice::from_ref(&a), GSS_CLOCK_QUERY, &mut g));
        assert_eq!(&g[4..8], &4u32.to_le_bytes());
        assert_eq!(&g[0x14..0x18], &0x000f_55c8u32.to_le_bytes());
        assert_eq!(g[0x210], 2);
        assert_eq!(
            g[0x30], 0x5a,
            "a byte the host did not write is the guest's"
        );
        // ⊘ Another domain, another size, another control: not ours.
        let mut other = row.request();
        other[0x0c] = 7;
        assert!(!answer(
            std::slice::from_ref(&a),
            GSS_CLOCK_QUERY,
            &mut other
        ));
        assert!(!answer(
            std::slice::from_ref(&a),
            GSS_CLOCK_QUERY,
            &mut [0; 16]
        ));
        assert!(!answer(&[a], GSS_CLOCK_DOMAINS, &mut row.request()));
    }

    /// ★ v3-refusals: the measured cudart clock pair — a host that writes a `u8` at 4 and full
    /// `u32`s at 8, 0x0c, 0x1c and 0x2c. Through the two probes the guest's uninitialised bytes
    /// ABOVE the `u8` survive (bare metal keeps them) while those inside the `u32`s do not.
    #[test]
    fn the_cudart_clock_query_is_answered_whole_fields_from_two_probes() {
        let row = *ROWS
            .iter()
            .find(|r| r.cmd == GSS_CUDART_CLOCKS)
            .expect("row");
        let host = |mut p: Vec<u8>| {
            p[4] = 2;
            p[8..12].copy_from_slice(&4u32.to_le_bytes());
            p[12..16].copy_from_slice(&1u32.to_le_bytes());
            p[0x1c..0x20].copy_from_slice(&1_695_000u32.to_le_bytes());
            p[0x2c..0x30].copy_from_slice(&9_751_000u32.to_le_bytes());
            p
        };
        let (a, used) = Answer::from_probes(
            row,
            &host(row.request()),
            Some(&host(row.request_over(PROBE_BACKGROUND))),
        );
        assert!(used, "the background did not change the answer");
        // The guest's request as bare-metal cudart builds it (a measured sample): garbage above
        // the byte at 4, in all of 0x0c, and from 0x34 on.
        let mut g = row.request_over(0x5a);
        g[0x08..0x0c].fill(0);
        for &(o, v) in row.inputs {
            g[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert!(answer(std::slice::from_ref(&a), GSS_CUDART_CLOCKS, &mut g));
        assert_eq!(g[4], 2);
        assert_eq!(
            &g[5..8],
            &[0x5a; 3],
            "bytes the host never writes stay the guest's"
        );
        assert_eq!(
            &g[12..16],
            &1u32.to_le_bytes(),
            "a whole u32 the host writes: no garbage survives above its low byte"
        );
        assert_eq!(&g[0x1c..0x20], &1_695_000u32.to_le_bytes());
        assert_eq!(&g[0x2c..0x30], &9_751_000u32.to_le_bytes());
        assert_eq!(g[0x40], 0x5a);
        // The one-probe answer is exactly the defect the second probe exists for.
        let narrow = Answer::from_host(row, &host(row.request()));
        let mut g2 = row.request_over(0x5a);
        for &(o, v) in row.inputs {
            g2[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert!(answer(
            std::slice::from_ref(&narrow),
            GSS_CUDART_CLOCKS,
            &mut g2
        ));
        assert_ne!(
            &g2[12..16],
            &1u32.to_le_bytes(),
            "one zero probe leaves 0x5a5a5a01"
        );
        // A request that differs in a named input is not this row's.
        let mut other = row.request();
        other[0x14] = 7;
        assert!(!answer(
            std::slice::from_ref(&a),
            GSS_CUDART_CLOCKS,
            &mut other
        ));
    }

    /// ★ v3-refusals: a host whose answer depends on the background is not trusted past the zero
    /// probe — the answer is never WIDER than the pre-v3-refusals rule.
    #[test]
    fn a_background_that_changes_the_answer_falls_back_to_the_zero_probe() {
        let row = *ROWS
            .iter()
            .find(|r| r.cmd == GSS_CUDART_CLOCKS)
            .expect("row");
        let mut zero = row.request();
        zero[8] = 4;
        let mut bg = row.request_over(PROBE_BACKGROUND);
        bg[8] = 9;
        let (a, used) = Answer::from_probes(row, &zero, Some(&bg));
        assert!(!used);
        assert_eq!(a, Answer::from_host(row, &zero));
        let (a, used) = Answer::from_probes(row, &zero, None);
        assert!(!used);
        assert_eq!(a.wrote, vec![(8, 4)]);
    }

    /// ★ v3-refusals: `0x2080a084` — the host's `NV_OK` with nothing written is an answer too.
    #[test]
    fn the_cudart_precursor_is_answered_with_nothing_written() {
        let row = *ROWS.iter().find(|r| r.cmd == GSS_CUDART_A084).expect("row");
        let (a, used) = Answer::from_probes(
            row,
            &row.request(),
            Some(&row.request_over(PROBE_BACKGROUND)),
        );
        assert!(used);
        assert!(a.wrote.is_empty());
        let mut g = [0u8; 4];
        assert!(answer(std::slice::from_ref(&a), GSS_CUDART_A084, &mut g));
        assert_eq!(g, [0; 4]);
        assert!(
            !answer(std::slice::from_ref(&a), GSS_CUDART_A084, &mut [1, 0, 0, 0]),
            "a non-zero request is not the row's"
        );
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
        assert!(
            !ROWS[0].matches(GSS_CLOCK_DOMAINS, &cudart),
            "cudart's request stays cudartinit's"
        );
    }
}
