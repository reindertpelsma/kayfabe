//! ★ **GPU utilisation reported to the guest** (`docs/design/V3_UTILIZATION_FLAG.md`, device property
//! `report-utilization`): the layouts of the three RM controls that carry it, GENERATED from NVIDIA's open
//! headers, and the pure encoders/decoders over them.
//!
//! STATUS: LIVE, 2026-11-17 (GPU-free; not run on hardware).
//!
//! Where the numbers come from: `tools/util-probe/probe.c` is compiled against each ogkm tag's SDK headers
//! (`tools/util-probe/generate.py`), its output is committed as one receipt per tag
//! (`traces/util_layout_20261117/`) and emitted as [`generated::ROWS`]. **Nothing in this file names an offset,
//! a size or a control id**: every one is read from a [`Layout`] row. The rule is the matrix's own, exact
//! membership: a driver version with no row has no layout, and the caller refuses by name (`layout` returns
//! `None`), never a nearest neighbour. That matters here, because the RUSD record is NOT stable across tags
//! (`perfDevUtil` moves from 1976 to 1992 at 590.48.01).
//!
//! Three controls (`design doc`, section 1):
//! - `NV2080_CTRL_CMD_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2`: the host read (a fixed request, built by
//!   [`host_request`]) and the guest reply ([`encode_gpumon_reply`]);
//! - `NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA` and `..._USER_SHARED_DATA_SET_DATA_POLL`: the guest's RUSD
//!   page and poll mask ([`parse_init`], [`parse_poll`]); the record kayfabe writes into that page is
//!   [`encode_rusd_record`].
//!
//! ⊘ Never here: a guest byte copied into a reply, a host byte copied into a reply other than the six reduced
//! percentages, a process id, a process name, a pointer.

use crate::DriverVersion;
use crate::generated::matrix as m;

#[path = "utilization_generated.rs"]
mod generated;

pub use generated::{Layout, ROWS};

/// The page size the RUSD record must stay inside (the guest's `RUSD` memdesc is one contiguous sysmem object;
/// kayfabe writes only inside its first page).
pub const PAGE: u64 = 4096;

/// How many engines the percentages cover: graphics/compute (`gpu`), memory bandwidth, NVENC, NVDEC, NVJPG, NVOFA.
pub const FIELDS: usize = 6;

/// The six reduced utilisation percentages, each already clamped to `0..=100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Percents {
    /// Graphics/compute engine busy (the host's `gr`), what NVML calls `utilization.gpu`.
    pub gpu: u8,
    /// Memory bandwidth busy (the host's `fb`), `utilization.memory`.
    pub memory: u8,
    /// NVENC busy.
    pub nvenc: u8,
    /// NVDEC busy.
    pub nvdec: u8,
    /// NVJPG busy.
    pub nvjpg: u8,
    /// NVOFA busy.
    pub nvofa: u8,
}

impl Percents {
    /// The values in the order `[gpu, memory, nvenc, nvdec, nvjpg, nvofa]`.
    #[must_use]
    pub fn array(&self) -> [u8; FIELDS] {
        [
            self.gpu,
            self.memory,
            self.nvenc,
            self.nvdec,
            self.nvjpg,
            self.nvofa,
        ]
    }
}

/// Reduce a control's `util` (units of percent x 100) to a whole percent in `0..=100`: rounded to nearest, any
/// value above 100 % clamped. Returns whether it was clamped.
#[must_use]
pub fn reduce(util_x100: u32) -> (u8, bool) {
    let pct = (u64::from(util_x100) + 50) / 100;
    if pct > 100 {
        (100, true)
    } else {
        // `pct <= 100`
        (u8::try_from(pct).unwrap_or(100), false)
    }
}

/// What one host reading carries: the host sample's own timestamp (used only to tell that it advanced; it is
/// never shown to the guest) and the six reduced percentages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostReading {
    /// The newest sample's `base.timeStamp`.
    pub host_timestamp: u64,
    /// The six percentages.
    pub percents: Percents,
    /// How many of the six raw values were above 100 % and clamped.
    pub clamped: u8,
}

/// The tag-exact row for `version`, or `None` (no row = refused by name).
#[must_use]
pub fn layout(version: DriverVersion) -> Option<&'static Layout> {
    ROWS.iter()
        .find(|(versions, _)| versions.contains(&version))
        .map(|(_, l)| l)
}

fn off(x: u64) -> usize {
    usize::try_from(x).unwrap_or(usize::MAX)
}

impl Layout {
    /// `sizeof` the whole GPUMON params, as a count of bytes.
    #[must_use]
    pub fn params_len(&self) -> usize {
        off(self.params_size)
    }

    /// `bufSize`: the sample buffer's size (`sample_size x sample_count`).
    #[must_use]
    pub fn buffer_size(&self) -> u64 {
        self.sample_size * self.sample_count
    }

    /// The GPUMON row's own consistency: the buffer ends where the params end, every engine record sits inside
    /// a sample, and the fields this crate writes are where the compiler says.
    #[must_use]
    pub fn gpumon_is_coherent(&self) -> bool {
        self.off_samples + self.buffer_size() == self.params_size
            && [
                self.off_fb,
                self.off_gr,
                self.off_nvenc,
                self.off_nvdec,
                self.off_nvjpg,
                self.off_nvofa,
            ]
            .iter()
            .all(|o| o + self.engine_size <= self.sample_size)
            && self.off_timestamp + 8 <= self.sample_size
            && self.eng_off_proc_id + 8 <= self.engine_size
            && self.eng_off_util + 8 <= self.engine_size
    }

    /// The RUSD row's own consistency: the record kayfabe writes (timestamp + info) lies wholly inside the first
    /// page of the object and inside the object, INIT and POLL are the 16-byte shapes parsed here, and the
    /// engine records the encoder fills fit the info block.
    #[must_use]
    pub fn rusd_is_coherent(&self) -> bool {
        let end = self.rusd_off_util + self.util_size;
        end <= self.rusd_size
            && end <= PAGE
            && self.util_off_ts + 8 <= self.util_size
            && self.info_off_eng + self.eng_count * self.eng_util_size
                <= self.util_size - self.util_off_info
            && self.info_off_gpu + 4 <= self.util_size
            && self.info_off_mem + 4 <= self.util_size
            && self.init_size == 16
            && self.init_off_binit < self.init_size
            && self.init_off_phys + 8 <= self.init_size
            && self.poll_size == 16
            && self.poll_off_mask + 8 <= self.poll_size
            && self.rusd_class == 0xde
    }

    /// Whether the driver matrix's own `PERFMON_UTIL_BUFFER_SIZE` value at `version` (`generated::matrix`,
    /// compiled per tag by `tools/drivermatrix`) equals the buffer this row derives (two independent compilations of
    /// one number).
    #[must_use]
    pub fn matrix_agrees(&self, version: DriverVersion) -> bool {
        m::CTRL_LIMITS_NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_BUFFER_SIZE
            .at(version)
            .ok()
            .flatten()
            == Some(self.buffer_size())
    }
}

/// The row to serve GPUMON at `version`: a row, coherent, and the matrix agrees on the buffer size. Anything
/// else is `None`.
#[must_use]
pub fn gpumon_at(version: DriverVersion) -> Option<&'static Layout> {
    layout(version).filter(|l| l.gpumon_is_coherent() && l.matrix_agrees(version))
}

/// The row to serve the RUSD controls at `version`: a row, coherent. Anything else is `None`.
#[must_use]
pub fn rusd_at(version: DriverVersion) -> Option<&'static Layout> {
    layout(version).filter(|l| l.rusd_is_coherent())
}

/// The row to READ THE HOST with: the host driver's own probed row, or, for a host inside the interval the host
/// encoders are written for (`[580.65.06, 581)`, [`crate::host_driver`]) that has no row of its own, the first
/// probed row inside that interval. That is sound because every probed row inside the interval prints the same
/// GPUMON layout (a test compares every field), and it is the interval rule the host controls already follow.
/// A host outside the interval with no row of its own gets `None` (refused by name).
#[must_use]
pub fn host_layout(host: DriverVersion) -> Option<&'static Layout> {
    let in_interval = |v: &DriverVersion| {
        crate::host_driver::HostDriverVersion {
            major: v.major,
            minor: v.minor,
            patch: v.patch,
        }
        .is_encoded_for()
    };
    layout(host).filter(|l| l.gpumon_is_coherent()).or_else(|| {
        in_interval(&host)
            .then(|| {
                ROWS.iter()
                    .find(|(vs, l)| vs.iter().any(in_interval) && l.gpumon_is_coherent())
                    .map(|(_, l)| l)
            })
            .flatten()
    })
}

/// The GPUMON control id, identical at every probed tag (a test pins it).
pub const GPUMON_CMD: u32 = ROWS[0].1.gpumon_cmd as u32;
/// The host-side params size (the HOST read is only ever made at layouts that print this size; a test pins that
/// every probed row prints it).
pub const HOST_PARAMS_SIZE: usize = ROWS[0].1.params_size as usize;
/// Host driver tags OUTSIDE `[580.65.06, 581)` at which a row was probed and prints [`HOST_PARAMS_SIZE`]
/// (a test checks each against its row and against the measured matrix).
pub const HOST_TAGS_BEYOND_580: &[DriverVersion] = &[
    DriverVersion {
        major: 590,
        minor: 48,
        patch: 1,
    },
    DriverVersion {
        major: 595,
        minor: 84,
        patch: 0,
    },
    DriverVersion {
        major: 595,
        minor: 91,
        patch: 7,
    },
    DriverVersion {
        major: 610,
        minor: 43,
        patch: 2,
    },
    DriverVersion {
        major: 610,
        minor: 57,
        patch: 4,
    },
    DriverVersion {
        major: 615,
        minor: 71,
        patch: 9,
    },
];

fn put_u32(buf: &mut [u8], at: usize, v: u32) -> Option<()> {
    buf.get_mut(at..at.checked_add(4)?)?
        .copy_from_slice(&v.to_le_bytes());
    Some(())
}

fn put_u64(buf: &mut [u8], at: usize, v: u64) -> Option<()> {
    buf.get_mut(at..at.checked_add(8)?)?
        .copy_from_slice(&v.to_le_bytes());
    Some(())
}

fn get_u32(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        buf.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn get_u64(buf: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        buf.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// ★ **The one host request**, host-authored and constant: zeroed params with `type = PERFMON_UTIL` and
/// `bufSize = sizeof(samples)`, exactly what the RM's own accounting builds (`gpu_acct.c:497-502`,
/// `docs/design/V3_UTILIZATION_FLAG.md` section 3). It depends on nothing the guest sent.
#[must_use]
pub fn host_request(l: &Layout) -> Vec<u8> {
    let mut p = vec![0u8; l.params_len()];
    // The offsets are `l`'s own and `gpumon_is_coherent` bounds them; a failed write cannot happen for a
    // coherent row, and an incoherent row gets an all-zero request that the host refuses (`bufSize` 0).
    let _ = p
        .get_mut(off(l.off_type))
        .map(|b| *b = u8::try_from(l.gpumon_type_util).unwrap_or(0));
    let _ = put_u32(
        &mut p,
        off(l.off_buf_size),
        u32::try_from(l.buffer_size()).unwrap_or(0),
    );
    p
}

fn engine_util(l: &Layout, sample: &[u8], eng_off: u64) -> Option<u32> {
    get_u32(sample, off(eng_off + l.eng_off_util))
}

/// Read the NEWEST sample out of a host reply and reduce it. "Newest" is the sample with the greatest
/// `timeStamp` among those with a non-zero one (the RM's own consumer picks by timestamp, `gpu_acct.c:553-565`),
/// so no ring-order rule is relied on. `None` when the reply is the wrong size, echoes another `type`/`bufSize`,
/// or holds no sample: **no sample is "not available", never zero.**
#[must_use]
pub fn decode_host_newest(l: &Layout, params: &[u8]) -> Option<HostReading> {
    if params.len() != l.params_len()
        || params.get(off(l.off_type)).copied() != u8::try_from(l.gpumon_type_util).ok()
        || u64::from(get_u32(params, off(l.off_buf_size))?) != l.buffer_size()
    {
        return None;
    }
    let base = off(l.off_samples);
    let (size, n) = (off(l.sample_size), off(l.sample_count));
    let mut newest: Option<(u64, &[u8])> = None;
    for i in 0..n {
        let s = params.get(base + i * size..base + (i + 1) * size)?;
        let ts = get_u64(s, off(l.off_timestamp))?;
        if ts != 0 && newest.is_none_or(|(t, _)| ts > t) {
            newest = Some((ts, s));
        }
    }
    let (host_timestamp, s) = newest?;
    let raw = [
        engine_util(l, s, l.off_gr)?,
        engine_util(l, s, l.off_fb)?,
        engine_util(l, s, l.off_nvenc)?,
        engine_util(l, s, l.off_nvdec)?,
        engine_util(l, s, l.off_nvjpg)?,
        engine_util(l, s, l.off_nvofa)?,
    ];
    let mut clamped = 0u8;
    let mut out = [0u8; FIELDS];
    for (o, r) in out.iter_mut().zip(raw) {
        let (pct, c) = reduce(r);
        *o = pct;
        clamped += u8::from(c);
    }
    Some(HostReading {
        host_timestamp,
        percents: Percents {
            gpu: out[0],
            memory: out[1],
            nvenc: out[2],
            nvdec: out[3],
            nvjpg: out[4],
            nvofa: out[5],
        },
        clamped,
    })
}

/// One sample of the GUEST reply: our own monotonic timestamp and the six percentages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestSample {
    /// Our own clock, nanoseconds (never a host value).
    pub timestamp_ns: u64,
    /// The percentages.
    pub percents: Percents,
}

/// Whether a guest GPUMON request is one this crate answers: the layout's size, `type` = PERFMON_UTIL and
/// `bufSize` = the buffer. **Nothing else of the request is read** (`count`, `tracker` and the buffer are
/// guest bytes; they are neither validated nor echoed).
#[must_use]
pub fn gpumon_request_is_served(l: &Layout, params: &[u8]) -> bool {
    params.len() == l.params_len()
        && params.get(off(l.off_type)).copied() == u8::try_from(l.gpumon_type_util).ok()
        && get_u32(params, off(l.off_buf_size)).map(u64::from) == Some(l.buffer_size())
}

/// ★ **The guest GPUMON reply params**, rebuilt from ZERO: `type`, `bufSize` (constants), `count`, `tracker`, and
/// up to `sample_count` samples oldest first at indices `0..count` (the header's "queue not full" shape: valid
/// entries between 0 and `tracker`, `tracker == count % sample_count`). Every process field of every engine is
/// the control's own "no process" value ([`Layout::pid_invalid`]); `util` is percent x 100 of the already
/// whole-percent value; names, pointers and the vGPU scale stay zero. Fixed size: always `params_len()` bytes.
///
/// `None` for an empty `samples` (nothing to say is "not available") or an incoherent row.
#[must_use]
pub fn encode_gpumon_reply(l: &Layout, samples: &[GuestSample]) -> Option<Vec<u8>> {
    if samples.is_empty() || !l.gpumon_is_coherent() {
        return None;
    }
    let n = off(l.sample_count);
    let samples = &samples[samples.len().saturating_sub(n)..];
    let mut p = vec![0u8; l.params_len()];
    *p.get_mut(off(l.off_type))? = u8::try_from(l.gpumon_type_util).ok()?;
    put_u32(
        &mut p,
        off(l.off_buf_size),
        u32::try_from(l.buffer_size()).ok()?,
    )?;
    let count = u32::try_from(samples.len()).ok()?;
    put_u32(&mut p, off(l.off_count), count)?;
    put_u32(
        &mut p,
        off(l.off_tracker),
        count % u32::try_from(l.sample_count).ok()?,
    )?;
    let pid = u32::try_from(l.pid_invalid).ok()?;
    for (i, s) in samples.iter().enumerate() {
        let at = off(l.off_samples) + i * off(l.sample_size);
        put_u64(&mut p, at + off(l.off_timestamp), s.timestamp_ns)?;
        let engines = [
            (l.off_gr, s.percents.gpu),
            (l.off_fb, s.percents.memory),
            (l.off_nvenc, s.percents.nvenc),
            (l.off_nvdec, s.percents.nvdec),
            (l.off_nvjpg, s.percents.nvjpg),
            (l.off_nvofa, s.percents.nvofa),
        ];
        for (eng_off, pct) in engines {
            let e = at + off(eng_off);
            put_u32(&mut p, e + off(l.eng_off_util), u32::from(pct) * 100)?;
            put_u32(&mut p, e + off(l.eng_off_proc_id), pid)?;
            put_u32(&mut p, e + off(l.eng_off_sub_proc_id), pid)?;
        }
    }
    Some(p)
}

/// A parsed `INIT_USER_SHARED_DATA` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RusdInit {
    /// `bInit = 1` with the guest's page address (validated: non-zero, page aligned, below 2^52).
    Link(u64),
    /// `bInit = 0`: unlink.
    Unlink,
}

/// Parse an INIT request (`None` = refuse: wrong size, `bInit` other than 0/1, or a link whose address is zero,
/// unaligned or at or above 2^52). The address is only stored by the caller; it is never dereferenced here.
#[must_use]
pub fn parse_init(l: &Layout, params: &[u8]) -> Option<RusdInit> {
    if params.len() != off(l.init_size) {
        return None;
    }
    match *params.get(off(l.init_off_binit))? {
        0 => Some(RusdInit::Unlink),
        1 => {
            let phys = get_u64(params, off(l.init_off_phys))?;
            (phys != 0 && phys % PAGE == 0 && phys < (1u64 << 52)).then_some(RusdInit::Link(phys))
        }
        _ => None,
    }
}

/// Parse a SET_DATA_POLL request: the mask and nothing else (the guest's interval is never read). `None` when
/// the size is wrong.
#[must_use]
pub fn parse_poll(l: &Layout, params: &[u8]) -> Option<u64> {
    if params.len() != off(l.poll_size) {
        return None;
    }
    get_u64(params, off(l.poll_off_mask))
}

/// Whether `mask` asks for the polled utilisation record (`NV00DE_RUSD_POLL_PERF`). Every other bit is ignored.
#[must_use]
pub fn mask_wants_utilization(l: &Layout, mask: u64) -> bool {
    mask & l.poll_perf != 0
}

/// What kayfabe writes into the guest's RUSD page, and where: `(offset from the page base, bytes)`, in the
/// order they must be written. The record's timestamp is the seqlock of `cl00de.h` (`WRITE_IN_PROGRESS`, then the
/// data, then a valid timestamp: non-zero, below `RUSD_SEQ_START`, never `WRITE_IN_PROGRESS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RusdWrites {
    /// Step 1: the timestamp word set to `WRITE_IN_PROGRESS`.
    pub begin: (u64, [u8; 8]),
    /// Step 2: the info block (gpu, memory, four engine records).
    pub info: (u64, Vec<u8>),
    /// Step 3: the timestamp word set to the valid value.
    pub finish: (u64, [u8; 8]),
}

/// Build the three writes for one publication. `timestamp` must be valid for the seqlock (checked: `None` when it
/// is 0, `WRITE_IN_PROGRESS` or at/above `RUSD_SEQ_START`); `period_us` is the sampler period the engine records
/// report as `samplingPeriodUs`. Every byte of the info block is authored here from `p` and `period_us`.
#[must_use]
pub fn encode_rusd_record(
    l: &Layout,
    p: &Percents,
    timestamp: u64,
    period_us: u32,
) -> Option<RusdWrites> {
    const SEQ_START: u64 = 0xFF00_0000_0000_0000;
    if !l.rusd_is_coherent()
        || timestamp == l.ts_invalid
        || timestamp == l.ts_in_progress
        || timestamp >= SEQ_START
    {
        return None;
    }
    let base = l.rusd_off_util;
    let info_len = off(l.util_size - l.util_off_info);
    let mut info = vec![0u8; info_len];
    put_u32(&mut info, off(l.info_off_gpu), u32::from(p.gpu))?;
    put_u32(&mut info, off(l.info_off_mem), u32::from(p.memory))?;
    let engines = [
        (l.eng_nvenc, p.nvenc),
        (l.eng_nvdec, p.nvdec),
        (l.eng_nvjpg, p.nvjpg),
        (l.eng_nvofa, p.nvofa),
    ];
    for (idx, pct) in engines {
        let e = off(l.info_off_eng + idx * l.eng_util_size);
        put_u32(&mut info, e + off(l.eng_util_off_busy), u32::from(pct))?;
        put_u32(&mut info, e + off(l.eng_util_off_period), period_us)?;
    }
    Some(RusdWrites {
        begin: (base + l.util_off_ts, l.ts_in_progress.to_le_bytes()),
        info: (base + l.util_off_info, info),
        finish: (base + l.util_off_ts, timestamp.to_le_bytes()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u16, minor: u16, patch: u16) -> DriverVersion {
        DriverVersion {
            major,
            minor,
            patch,
        }
    }

    fn l580() -> &'static Layout {
        layout(v(580, 159, 4)).expect("the pinned oracle's row")
    }

    #[test]
    fn every_row_is_coherent_and_the_matrix_agrees_at_every_version_it_covers() {
        for (versions, l) in ROWS {
            assert!(l.gpumon_is_coherent(), "{versions:?}");
            assert!(l.rusd_is_coherent(), "{versions:?}");
            for ver in *versions {
                assert!(
                    l.matrix_agrees(*ver),
                    "{ver}: the matrix's measured buffer size differs from the derived one"
                );
                assert!(m::MEASURED.contains(ver), "{ver} is not a measured tag");
            }
        }
    }

    #[test]
    fn the_record_kayfabe_writes_stays_inside_the_first_page_at_every_row() {
        for (versions, l) in ROWS {
            let w = encode_rusd_record(l, &Percents::default(), 1, 250_000).expect("coherent");
            for (at, len) in [
                (w.begin.0, 8),
                (w.info.0, w.info.1.len() as u64),
                (w.finish.0, 8),
            ] {
                assert!(
                    at + len <= PAGE,
                    "{versions:?}: write {at}+{len} leaves the page"
                );
                assert!(at >= l.rusd_off_util, "{versions:?}");
                assert!(
                    at + len <= l.rusd_off_util + l.util_size,
                    "{versions:?}: write {at}+{len} leaves perfDevUtil"
                );
            }
        }
    }

    #[test]
    fn the_rusd_record_moves_between_tags_and_exact_membership_says_so() {
        // 580.159.04 and 595.84 print different offsets for the same field: a nearest-neighbour rule would
        // write into the wrong place for one of them.
        assert_eq!(layout(v(580, 159, 4)).unwrap().rusd_off_util, 1976);
        assert_eq!(layout(v(595, 84, 0)).unwrap().rusd_off_util, 1992);
        // A tag nobody probed has no row (570.x has an older INIT struct, 580.65.1 is not a release).
        assert!(layout(v(570, 86, 15)).is_none());
        assert!(layout(v(580, 65, 1)).is_none());
        assert!(layout(v(581, 0, 0)).is_none());
    }

    #[test]
    fn every_row_agrees_on_the_gpumon_control_and_the_host_size() {
        for (_, l) in ROWS {
            assert_eq!(l.gpumon_cmd as u32, GPUMON_CMD);
            assert_eq!(l.params_len(), HOST_PARAMS_SIZE);
        }
        assert_eq!(GPUMON_CMD, 0x2080_2096);
        assert_eq!(HOST_PARAMS_SIZE, 55_888);
    }

    #[test]
    fn every_row_prints_the_same_gpumon_layout_so_an_unprobed_host_in_the_interval_may_borrow_one()
    {
        let first = ROWS[0].1;
        let gpumon = |l: &Layout| {
            [
                l.gpumon_cmd,
                l.gpumon_type_util,
                l.pid_invalid,
                l.sample_count,
                l.params_size,
                l.off_type,
                l.off_buf_size,
                l.off_count,
                l.off_tracker,
                l.off_samples,
                l.sample_size,
                l.off_timestamp,
                l.off_fb,
                l.off_gr,
                l.off_nvenc,
                l.off_nvdec,
                l.off_nvjpg,
                l.off_nvofa,
                l.engine_size,
                l.eng_off_util,
                l.eng_off_vgpu_scale,
                l.eng_off_proc_id,
                l.eng_off_sub_proc_id,
            ]
        };
        for (versions, l) in ROWS {
            assert_eq!(gpumon(l), gpumon(&first), "{versions:?}");
        }
    }

    #[test]
    fn the_host_row_is_exact_inside_the_interval_borrowed_for_an_unprobed_patch_and_refused_outside()
     {
        assert!(host_layout(v(580, 159, 4)).is_some(), "its own row");
        assert!(
            host_layout(v(580, 142, 3)).is_some(),
            "no row of its own, inside [580.65.06, 581)"
        );
        assert!(
            host_layout(v(580, 65, 5)).is_none(),
            "580.65.05 is below the interval and has no row"
        );
        assert!(host_layout(v(581, 0, 0)).is_none());
        assert!(host_layout(v(535, 309, 1)).is_none(), "no row, outside");
        assert!(
            host_layout(v(595, 91, 7)).is_some(),
            "outside the interval but probed"
        );
        assert!(host_layout(v(595, 91, 8)).is_none(), "outside and unprobed");
    }

    #[test]
    fn the_host_tags_listed_beyond_580_have_rows_that_print_the_host_size() {
        for t in HOST_TAGS_BEYOND_580 {
            let l = layout(*t).unwrap_or_else(|| panic!("{t} has no probed row"));
            assert_eq!(l.params_len(), HOST_PARAMS_SIZE, "{t}");
            assert!(m::MEASURED.contains(t), "{t}");
            assert!(l.matrix_agrees(*t), "{t}");
        }
    }

    /// A host reply with `n` samples, `ts(i)` stamps and the same `util` in the six engines.
    fn host_reply(l: &Layout, stamps: &[u64], util: [u32; 6]) -> Vec<u8> {
        let mut p = host_request(l);
        let engines = [
            l.off_gr,
            l.off_fb,
            l.off_nvenc,
            l.off_nvdec,
            l.off_nvjpg,
            l.off_nvofa,
        ];
        for (i, ts) in stamps.iter().enumerate() {
            let at = off(l.off_samples) + i * off(l.sample_size);
            put_u64(&mut p, at + off(l.off_timestamp), *ts).unwrap();
            for (e, u) in engines.iter().zip(util) {
                put_u32(&mut p, at + off(*e) + off(l.eng_off_util), u).unwrap();
            }
        }
        p
    }

    #[test]
    fn the_host_request_is_constant_and_says_perfmon_util() {
        let l = l580();
        let r = host_request(l);
        assert_eq!(r.len(), 55_888);
        assert_eq!(r[0], 2, "NV2080_CTRL_GPUMON_SAMPLE_TYPE_PERFMON_UTIL");
        assert_eq!(get_u32(&r, 4), Some(55_872));
        assert_eq!(
            r.iter().filter(|b| **b != 0).count(),
            3,
            "type 2 and bufSize 55872 (0xda40) only"
        );
        assert_eq!(host_request(l), r, "no input, so no variation");
    }

    #[test]
    fn the_newest_sample_wins_by_timestamp_and_values_reduce_to_whole_percent() {
        let l = l580();
        let mut p = host_reply(l, &[5, 9, 7], [10_000, 5_049, 5_050, 0, 1, 99]);
        // make sample 1 (ts 9) the one that is read: give it distinct values
        let at = off(l.off_samples) + off(l.sample_size);
        put_u32(&mut p, at + off(l.off_gr) + off(l.eng_off_util), 4_200).unwrap();
        let r = decode_host_newest(l, &p).expect("a sample");
        assert_eq!(r.host_timestamp, 9);
        assert_eq!(r.percents.gpu, 42);
        assert_eq!(r.percents.memory, 50, "5049 rounds to 50");
        assert_eq!(r.percents.nvenc, 51, "5050 rounds up");
        assert_eq!(r.percents.nvdec, 0);
        assert_eq!(r.percents.nvjpg, 0, "1 -> 0");
        assert_eq!(r.percents.nvofa, 1, "99 -> 1");
        assert_eq!(r.clamped, 0);
    }

    #[test]
    fn a_value_above_100_percent_is_clamped_and_counted() {
        let l = l580();
        let r = decode_host_newest(
            l,
            &host_reply(l, &[1], [10_049, 10_050, u32::MAX, 20_000, 10_000, 0]),
        )
        .unwrap();
        assert_eq!(r.percents.array(), [100, 100, 100, 100, 100, 0]);
        assert_eq!(
            r.clamped, 3,
            "10049 and 10000 round to 100 and are NOT clamped; 10050 (101), MAX and 20000 are"
        );
    }

    #[test]
    fn a_reply_with_no_sample_or_the_wrong_shape_is_not_available_not_zero() {
        let l = l580();
        assert!(
            decode_host_newest(l, &host_reply(l, &[], [5; 6])).is_none(),
            "empty ring"
        );
        assert!(
            decode_host_newest(l, &host_reply(l, &[0, 0], [5; 6])).is_none(),
            "all timestamps zero"
        );
        assert!(decode_host_newest(l, &host_request(l)).is_none());
        let mut bad_type = host_reply(l, &[1], [5; 6]);
        bad_type[0] = 1;
        assert!(decode_host_newest(l, &bad_type).is_none());
        let mut bad_size = host_reply(l, &[1], [5; 6]);
        put_u32(&mut bad_size, 4, 12).unwrap();
        assert!(decode_host_newest(l, &bad_size).is_none());
        assert!(
            decode_host_newest(l, &host_reply(l, &[1], [5; 6])[1..]).is_none(),
            "truncated"
        );
    }

    #[test]
    fn the_guest_reply_is_rebuilt_from_zero_with_no_process_and_fixed_size() {
        let l = l580();
        let s = |t, g| GuestSample {
            timestamp_ns: t,
            percents: Percents {
                gpu: g,
                memory: 7,
                nvenc: 0,
                nvdec: 3,
                nvjpg: 0,
                nvofa: 100,
            },
        };
        let p = encode_gpumon_reply(l, &[s(10, 40), s(20, 41)]).unwrap();
        assert_eq!(p.len(), l.params_len());
        assert_eq!(p[0], 2);
        assert_eq!(get_u32(&p, 4), Some(55_872));
        assert_eq!(get_u32(&p, 8), Some(2), "count");
        assert_eq!(get_u32(&p, 12), Some(2), "tracker");
        let at = off(l.off_samples);
        assert_eq!(get_u64(&p, at), Some(10));
        assert_eq!(get_u32(&p, at + off(l.off_gr)), Some(4_000), "pct x 100");
        assert_eq!(
            get_u32(&p, at + off(l.off_gr) + 8),
            Some(u32::MAX),
            "procId = NV2080_GPUMON_PID_INVALID"
        );
        assert_eq!(
            get_u32(&p, at + off(l.off_gr) + 12),
            Some(u32::MAX),
            "subProcessID"
        );
        assert_eq!(get_u32(&p, at + off(l.off_nvofa)), Some(10_000));
        // sample 2 is where sample 2 goes; everything after it is zero
        assert_eq!(get_u64(&p, at + off(l.sample_size)), Some(20));
        assert!(p[at + 2 * off(l.sample_size)..].iter().all(|b| *b == 0));
        // names and pointers of the engine record are zero
        let e = at + off(l.off_gr);
        assert!(p[e + 16..e + 128].iter().all(|b| *b == 0));
    }

    #[test]
    fn a_full_ring_keeps_the_newest_72_and_points_the_tracker_at_the_oldest() {
        let l = l580();
        let many: Vec<GuestSample> = (1..=100)
            .map(|i| GuestSample {
                timestamp_ns: i,
                percents: Percents::default(),
            })
            .collect();
        let p = encode_gpumon_reply(l, &many).unwrap();
        assert_eq!(get_u32(&p, 8), Some(72));
        assert_eq!(
            get_u32(&p, 12),
            Some(0),
            "tracker 0 = the oldest entry is index 0"
        );
        assert_eq!(get_u64(&p, off(l.off_samples)), Some(29), "100 - 72 + 1");
    }

    #[test]
    fn nothing_to_say_is_not_a_reply() {
        assert!(encode_gpumon_reply(l580(), &[]).is_none());
    }

    #[test]
    fn a_guest_request_is_served_only_at_the_exact_shape_and_nothing_else_is_read() {
        let l = l580();
        let mut r = host_request(l);
        assert!(gpumon_request_is_served(l, &r));
        // count, tracker and the buffer are guest bytes: any value is served the same
        put_u32(&mut r, 8, 0xdead_beef).unwrap();
        put_u32(&mut r, 12, 0xffff_ffff).unwrap();
        r[20] = 0xaa;
        assert!(gpumon_request_is_served(l, &r));
        let reply_a = encode_gpumon_reply(
            l,
            &[GuestSample {
                timestamp_ns: 1,
                percents: Percents::default(),
            }],
        );
        assert_eq!(
            reply_a,
            encode_gpumon_reply(
                l,
                &[GuestSample {
                    timestamp_ns: 1,
                    percents: Percents::default()
                }]
            ),
            "the reply is a function of the snapshot only"
        );
        let mut bad = host_request(l);
        bad[0] = 1;
        assert!(!gpumon_request_is_served(l, &bad));
        let mut bad = host_request(l);
        put_u32(&mut bad, 4, 55_871).unwrap();
        assert!(!gpumon_request_is_served(l, &bad));
        assert!(!gpumon_request_is_served(l, &host_request(l)[..55_887]));
    }

    fn init(binit: u8, phys: u64) -> Vec<u8> {
        let mut p = vec![0u8; 16];
        p[0] = binit;
        p[8..16].copy_from_slice(&phys.to_le_bytes());
        p
    }

    #[test]
    fn init_links_only_a_valid_page_and_unlinks_on_zero() {
        let l = l580();
        assert_eq!(
            parse_init(l, &init(1, 0x1_2345_6000)),
            Some(RusdInit::Link(0x1_2345_6000))
        );
        assert_eq!(parse_init(l, &init(0, 0)), Some(RusdInit::Unlink));
        assert_eq!(
            parse_init(l, &init(0, 0xdead)),
            Some(RusdInit::Unlink),
            "address ignored on unlink"
        );
        for bad in [
            init(1, 0),
            init(1, 0x1001),
            init(1, 1 << 52),
            init(2, 0x1000),
            init(0xff, 0x1000),
        ] {
            assert_eq!(parse_init(l, &bad), None, "{bad:?}");
        }
        assert_eq!(parse_init(l, &init(1, 0x1000)[..15]), None);
        let mut long = init(1, 0x1000);
        long.push(0);
        assert_eq!(parse_init(l, &long), None);
    }

    #[test]
    fn poll_reads_the_mask_and_only_the_perf_bit_matters() {
        let l = l580();
        let mut p = vec![0u8; 16];
        p[0..8].copy_from_slice(&0xffff_ffff_ffff_ffffu64.to_le_bytes());
        p[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(parse_poll(l, &p), Some(u64::MAX));
        assert!(mask_wants_utilization(l, u64::MAX));
        assert!(mask_wants_utilization(l, 2));
        for other in [0, 1, 4, 8, 0x10, 0x20, 0x40, !2u64] {
            assert!(!mask_wants_utilization(l, other), "{other:#x}");
        }
        assert_eq!(parse_poll(l, &p[..8]), None);
    }

    #[test]
    fn the_rusd_record_is_a_seqlock_write_at_the_generated_offsets() {
        let l = l580();
        let p = Percents {
            gpu: 91,
            memory: 12,
            nvenc: 1,
            nvdec: 2,
            nvjpg: 3,
            nvofa: 4,
        };
        let w = encode_rusd_record(l, &p, 77, 250_000).unwrap();
        assert_eq!(w.begin, (1976, u64::MAX.to_le_bytes()));
        assert_eq!(w.finish, (1976, 77u64.to_le_bytes()));
        assert_eq!(w.info.0, 1984);
        let i = &w.info.1;
        assert_eq!(i.len(), 40);
        assert_eq!(get_u32(i, 0), Some(91), "gpuPercentBusy");
        assert_eq!(get_u32(i, 4), Some(12), "memoryPercentBusy");
        for (k, want) in [1u32, 2, 3, 4].iter().enumerate() {
            assert_eq!(
                get_u32(i, 8 + 8 * k),
                Some(*want),
                "engUtil[{k}].clkPercentBusy"
            );
            assert_eq!(
                get_u32(i, 12 + 8 * k),
                Some(250_000),
                "engUtil[{k}].samplingPeriodUs"
            );
        }
        // 595.84 puts it elsewhere
        let w = encode_rusd_record(layout(v(595, 84, 0)).unwrap(), &p, 77, 1).unwrap();
        assert_eq!(w.begin.0, 1992);
    }

    #[test]
    fn a_timestamp_the_seqlock_would_misread_is_refused() {
        let l = l580();
        for ts in [0, u64::MAX, 0xFF00_0000_0000_0000, 0xFF00_0000_0000_0001] {
            assert!(
                encode_rusd_record(l, &Percents::default(), ts, 1).is_none(),
                "{ts:#x}"
            );
        }
        assert!(encode_rusd_record(l, &Percents::default(), 0xFEFF_FFFF_FFFF_FFFF, 1).is_some());
    }

    #[test]
    fn the_newest_sample_wins_whatever_order_the_host_ring_is_in() {
        let l = l580();
        for stamps in [[1u64, 2, 3], [3, 2, 1], [2, 3, 1]] {
            let mut p = host_reply(l, &stamps, [0; 6]);
            // the sample stamped 3 carries gpu 80, the others 1
            for (i, ts) in stamps.iter().enumerate() {
                let at = off(l.off_samples) + i * off(l.sample_size);
                let util = if *ts == 3 { 8_000 } else { 100 };
                put_u32(&mut p, at + off(l.off_gr) + off(l.eng_off_util), util).unwrap();
            }
            let r = decode_host_newest(l, &p).unwrap();
            assert_eq!((r.host_timestamp, r.percents.gpu), (3, 80), "{stamps:?}");
        }
    }

    #[test]
    fn a_host_reply_that_echoes_another_sample_type_is_not_read() {
        let l = l580();
        let mut p = host_reply(l, &[5], [1_000; 6]);
        assert!(decode_host_newest(l, &p).is_some());
        p[0] = 1;
        assert!(decode_host_newest(l, &p).is_none());
    }

    #[test]
    fn a_host_reply_that_echoes_another_buffer_size_is_not_read() {
        let l = l580();
        let mut p = host_reply(l, &[5], [1_000; 6]);
        put_u32(&mut p, 4, 0).unwrap();
        assert!(decode_host_newest(l, &p).is_none());
    }

    #[test]
    fn exactly_72_samples_put_the_tracker_at_zero_and_keep_every_one() {
        let l = l580();
        let exactly: Vec<GuestSample> = (1..=72)
            .map(|i| GuestSample {
                timestamp_ns: i,
                percents: Percents::default(),
            })
            .collect();
        let p = encode_gpumon_reply(l, &exactly).unwrap();
        assert_eq!(get_u32(&p, 8), Some(72));
        assert_eq!(get_u32(&p, 12), Some(0), "72 % 72");
        assert_eq!(
            get_u64(&p, off(l.off_samples)),
            Some(1),
            "the oldest of 72 is the first"
        );
        let one_more: Vec<GuestSample> = (1..=73)
            .map(|i| GuestSample {
                timestamp_ns: i,
                percents: Percents::default(),
            })
            .collect();
        let p = encode_gpumon_reply(l, &one_more).unwrap();
        assert_eq!(
            get_u64(&p, off(l.off_samples)),
            Some(2),
            "the newest 72 of 73"
        );
        assert_eq!(
            get_u64(&p, off(l.off_samples) + 71 * off(l.sample_size)),
            Some(73),
            "and the newest is last"
        );
    }

    #[test]
    fn a_timestamp_in_the_seqlocks_reserved_range_is_refused_whichever_edge() {
        let l = l580();
        assert!(encode_rusd_record(l, &Percents::default(), 0xFF00_0000_0000_0000, 1).is_none());
        assert!(encode_rusd_record(l, &Percents::default(), 0xFFFF_FFFF_FFFF_FFFE, 1).is_none());
        assert!(encode_rusd_record(l, &Percents::default(), l.ts_in_progress, 1).is_none());
        assert!(encode_rusd_record(l, &Percents::default(), l.ts_invalid, 1).is_none());
        assert!(encode_rusd_record(l, &Percents::default(), 1, 1).is_some());
    }

    #[test]
    fn the_seqlocks_first_write_is_never_a_valid_timestamp() {
        for (_, l) in ROWS {
            let w = encode_rusd_record(l, &Percents::default(), 12345, 1).unwrap();
            assert_eq!(u64::from_le_bytes(w.begin.1), l.ts_in_progress);
            assert_ne!(w.begin.1, w.finish.1);
            assert_eq!(u64::from_le_bytes(w.finish.1), 12345);
        }
    }

    /// `major.minor.patch` as the matrix and the receipts spell a tag (`580.159.04`, `595.84`).
    fn tag_name(v: DriverVersion) -> String {
        if v.patch == 0 {
            format!("{}.{}", v.major, v.minor)
        } else {
            format!("{}.{}.{:02}", v.major, v.minor, v.patch)
        }
    }

    /// ★ Security review of PR #459, F2: the tag set must not be a hand-typed list. Every driver version in the driver
    /// matrix's tag list (`generated::matrix`) from the first with the GPUMON v2 geometry up has either a layout row or a
    /// receipt that says why not (`unprobeable`); a listed version with neither fails here, so a missing row is a visible refusal and
    /// never a silent gap.
    #[test]
    fn every_measured_tag_has_a_row_or_an_unprobeable_receipt() {
        let sources = include_str!("../../../traces/util_layout_20261117/sources.json");
        let first = DriverVersion {
            major: 570,
            minor: 86,
            patch: 15,
        };
        let mut covered = 0;
        for v in m::MEASURED.iter().filter(|v| **v >= first) {
            let tag = tag_name(*v);
            let entry = sources
                .split(&format!("\"{tag}\": {{"))
                .nth(1)
                .unwrap_or_else(|| {
                    panic!("{tag} is a measured driver version with NO receipt in sources.json")
                })
                .split('}')
                .next()
                .unwrap();
            let has_row = layout(*v).is_some();
            let unprobeable = entry.contains("\"unprobeable\"");
            assert!(
                has_row != unprobeable,
                "{tag}: exactly one of a layout row ({has_row}) and an unprobeable receipt ({unprobeable})"
            );
            covered += 1;
        }
        assert!(covered >= 20, "{covered}");
        // the version the review found missing
        assert!(
            layout(DriverVersion {
                major: 580,
                minor: 178,
                patch: 4
            })
            .is_some()
        );
    }

    #[test]
    fn a_guest_version_with_no_row_is_refused_never_guessed() {
        // the one a neighbouring row would have lent: 580.178.04's neighbours are 580.173.02 and 590.48.01
        for (maj, min, pat) in [(580, 174, 0), (580, 179, 1), (590, 47, 9), (575, 51, 1)] {
            let v = DriverVersion {
                major: maj,
                minor: min,
                patch: pat,
            };
            assert!(layout(v).is_none(), "{v}");
            assert!(gpumon_at(v).is_none() && rusd_at(v).is_none(), "{v}");
        }
    }

    #[test]
    fn reduce_rounds_to_nearest_and_clamps() {
        assert_eq!(reduce(0), (0, false));
        assert_eq!(reduce(49), (0, false));
        assert_eq!(reduce(50), (1, false));
        assert_eq!(reduce(10_049), (100, false));
        assert_eq!(reduce(10_050), (100, true));
        assert_eq!(reduce(u32::MAX), (100, true));
    }
}
