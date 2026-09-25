//! ★ `0x2080a028` — the GSS-legacy PERF clock query `libnvidia-encode` gates an encode session on.
//!
//! ## Why it exists here
//!
//! `[measured 2026-09-26, vvid, GA106 / 580.159.04, the nvdiff LD_PRELOAD recorder]` bare-metal
//! `ffmpeg -c:v h264_nvenc` issues `0x2080a028` (paramsSize `0x890`) right after CUDA context
//! creation, gets `NV_OK`, and goes on to `GPU_GET_ENGINES_V2` / `GET_CLASSLIST` / the encoder
//! objects. In a kayfabe guest the same call was refused `NV_ERR_NOT_SUPPORTED` (unserviced) and
//! the library stopped THERE: *"OpenEncodeSessionEx failed: unsupported device (2)"* — the first
//! divergence of the host↔guest ioctl diff, every earlier record identical.
//!
//! Bit 15 set = the GSS-legacy family (`RM_GSS_LEGACY_MASK`): no `ogkm` header names it, the
//! guest's CPU-RM routes it to the GSP, and a GSP answers it. ⊘ So its layout is MEASURED, not
//! read from source — from one host request/reply pair, and every field below says so:
//!
//! ```text
//! pre   01000000 00000000 01000000 00001000 00000000 00000000 00000000 <uninitialised …>
//! post  01000000 04000000 01000000 00001000 00000000 c8550f00 00000000 <unchanged …>
//! ```
//!
//! read as `{u32 in0 = 1, u32 out = 4, u32 count = 1, entries[count] of 16 bytes {u32 domain =
//! 0x00100000, u32 0, u32 value = 0x000f55c8, u32 0}}`. `0x00100000` is the NVD clock domain
//! (`NV2080_CTRL_CLK_DOMAIN_NVDCLK` in NVIDIA's pre-open clock headers — the video-engine clock)
//! and `0x000f55c8` = 1 005 000, i.e. 1005 MHz in kHz. The host wrote ONLY `out` and the entry
//! (the uninitialised tail came back byte-identical).
//!
//! ## What this port does with it
//!
//! ⊘ Never forwarded. At realize the device asks the HOST the same question for each clock domain
//! it knows ([`CLOCK_DOMAINS`]), with a request it authors, and keeps the host's `out` word and
//! entry. A guest request is answered only when EVERY domain it names is one the host answered;
//! then exactly the bytes the host writes are written (`out`, and each entry) — anything else is
//! refused, as before. The value is the host's clock at VM start: a clock is a moving number and
//! this is a statement of capability, not telemetry.

/// The control id (bit 15 = GSS legacy).
pub const GSS_PERF_CLOCK_QUERY: u32 = 0x2080_a028;
/// `[measured]` the paramsSize bare-metal libnvidia-encode declares.
pub const PARAMS_SIZE: usize = 0x890;
/// `[measured]` the header: `in0`, `out`, `count`.
pub const HEADER: usize = 12;
/// `[measured]` one entry: `{domain, 0, value, 0}`.
pub const ENTRY: usize = 16;
/// The most entries the measured buffer holds.
pub const MAX_ENTRIES: usize = (PARAMS_SIZE - HEADER) / ENTRY;
/// The clock domains a device asks its host about at realize: the NVD (video) clock, the one the
/// encoder asks. `[measured]` only this one has been seen on the wire.
pub const CLOCK_DOMAINS: &[u32] = &[0x0010_0000];

/// What the host answered for one domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockAnswer {
    /// The domain asked.
    pub domain: u32,
    /// The header `out` word the host wrote.
    pub out: u32,
    /// The entry the host wrote back, verbatim.
    pub entry: [u8; ENTRY],
}

/// The request this port authors to ask the host about `domain`.
#[must_use]
pub fn host_request(domain: u32) -> Vec<u8> {
    let mut p = vec![0u8; PARAMS_SIZE];
    p[0..4].copy_from_slice(&1u32.to_le_bytes());
    p[8..12].copy_from_slice(&1u32.to_le_bytes());
    p[HEADER..HEADER + 4].copy_from_slice(&domain.to_le_bytes());
    p
}

/// Read the host's reply to [`host_request`]`(domain)`; `None` if it is not the measured shape
/// (count 1, the same domain echoed).
#[must_use]
pub fn decode_host_reply(domain: u32, reply: &[u8]) -> Option<ClockAnswer> {
    let w = |o: usize| reply.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if w(8)? != 1 || w(HEADER)? != domain {
        return None;
    }
    let mut entry = [0u8; ENTRY];
    entry.copy_from_slice(reply.get(HEADER..HEADER + ENTRY)?);
    Some(ClockAnswer { domain, out: w(4)?, entry })
}

/// Why a guest request was not answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockQueryRefusal {
    /// Not the measured size.
    Size(usize),
    /// A count of zero, or more entries than the buffer holds.
    Count(u32),
    /// A domain the host was never asked about (or did not answer).
    Domain(u32),
    /// The requested domains' host answers disagree on the header `out` word.
    Inconsistent,
}

/// ★ Answer a guest's `0x2080a028` params in place from the host's realize-time answers.
///
/// # Errors
/// [`ClockQueryRefusal`] — the caller refuses the control, as it did before this module.
pub fn answer(answers: &[ClockAnswer], params: &mut [u8]) -> Result<(), ClockQueryRefusal> {
    if params.len() != PARAMS_SIZE {
        return Err(ClockQueryRefusal::Size(params.len()));
    }
    let rd = |p: &[u8], o: usize| u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
    let count = rd(params, 8);
    if count == 0 || count as usize > MAX_ENTRIES {
        return Err(ClockQueryRefusal::Count(count));
    }
    let mut hits = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let d = rd(params, HEADER + i * ENTRY);
        hits.push(*answers.iter().find(|a| a.domain == d).ok_or(ClockQueryRefusal::Domain(d))?);
    }
    let out = hits[0].out;
    if hits.iter().any(|a| a.out != out) {
        return Err(ClockQueryRefusal::Inconsistent);
    }
    params[4..8].copy_from_slice(&out.to_le_bytes());
    for (i, a) in hits.iter().enumerate() {
        params[HEADER + i * ENTRY..HEADER + (i + 1) * ENTRY].copy_from_slice(&a.entry);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured host pair, reproduced: the request this port authors IS the bare-metal one
    /// (its first 28 bytes), and the answer lands where the host wrote.
    #[test]
    fn the_measured_pair_round_trips() {
        let req = host_request(0x0010_0000);
        assert_eq!(&req[..28], &[1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut host_reply = req.clone();
        host_reply[4] = 4;
        host_reply[20..24].copy_from_slice(&0x000f_55c8u32.to_le_bytes());
        let a = decode_host_reply(0x0010_0000, &host_reply).expect("measured shape");
        assert_eq!(a.out, 4);
        // A guest buffer with an uninitialised tail: the tail survives, the answer lands.
        let mut guest = req.clone();
        for b in &mut guest[28..] {
            *b = 0x5a;
        }
        answer(&[a], &mut guest).expect("answered");
        assert_eq!(&guest[..28], &host_reply[..28]);
        assert!(guest[28..].iter().all(|b| *b == 0x5a));
    }

    #[test]
    fn an_unasked_domain_or_bad_shape_is_refused() {
        let a = ClockAnswer { domain: 0x0010_0000, out: 4, entry: [0; ENTRY] };
        let mut p = host_request(0x1);
        assert_eq!(answer(&[a], &mut p), Err(ClockQueryRefusal::Domain(1)));
        let mut p = host_request(0x0010_0000);
        p[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(answer(&[a], &mut p), Err(ClockQueryRefusal::Count(0)));
        let mut short = vec![0u8; 16];
        assert_eq!(answer(&[a], &mut short), Err(ClockQueryRefusal::Size(16)));
        assert_eq!(decode_host_reply(0x2, &host_request(0x0010_0000)), None);
    }
}
