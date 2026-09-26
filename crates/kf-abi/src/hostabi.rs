//! ★★★ THE HOST DRIVER AXIS, MEASURED (`docs/design/V3_DRIVER_MATRIX.md` §2.2, §8.3).
//!
//! # The problem
//!
//! Every host-side RM struct kayfabe builds (`kf-host`, `kf-rm::hostquery`) was written against
//! ONE layout — the bench driver's, 580.159.04 — and [`crate::host_driver`] pinned the host to the
//! interval where that layout is true (`[580.65.06, 581)`). The matrix shows the same structs at
//! every other measured tag: `NVOS46` is 56 bytes below 580.65.06 (no `flags2`, no
//! `kindOverride`), `NVOS47` 40 bytes at 535/545, `GPFIFO_SCHEDULE` loses `bSkipEnable` at ≤570,
//! `NV_CHANNEL_ALLOC_PARAMS` is 360 bytes at ≤565 and gains `hHandleVASpace` at 610,
//! `GET_CLASSLIST_V2` holds 100 classes at 555–575 …
//!
//! # The rule: one encoder, carried at the boundary
//!
//! ⊘ Not a second set of encoders per version — that is the hand-row shape the matrix exists to
//! retire. The host side keeps speaking the BENCH layout internally; the struct is CARRIED to the
//! host's own measured layout on the way out and back on the way in, by field name, through the
//! same transcoder the guest axis uses ([`crate::matrix::transcode`]). What that buys, per struct:
//!
//! - **same layout** at the host's version (the common case inside a branch) ⇒ the bytes pass
//!   through untouched ([`HostAbi::carry`] returns [`Carry::Same`]);
//! - **moved fields** ⇒ carried by name;
//! - **a field the host's version does not have**, carrying a value ⇒ [`HostAbiError::Unexpressible`]:
//!   the host cannot be asked that (e.g. a kind override on a 575 host), and saying so by name is
//!   the only honest answer — the silent alternative is the host reading our field as its
//!   neighbour's;
//! - **a reply field only the host has** ⇒ dropped (reported), since the bench encoder cannot
//!   read it anyway.
//!
//! ⊘ **Exact membership, no nearest neighbour** — the matrix's own rule: a host driver that was
//! never measured is refused ([`HostAbiError::Unmeasured`]), because a release between two
//! measured tags is exactly where a borrowed layout is silently wrong.

use crate::DriverVersion;
use crate::host_driver::HostDriverVersion;
use crate::matrix::{LayoutError, Resolved, StructRuns, TranscodeError, is_measured, transcode};
use std::fmt;

/// The host driver's measured ABI: which layout each host-side struct has at that version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAbi {
    version: DriverVersion,
}

/// How a struct crosses the boundary at this host version.
#[derive(Debug, Clone, Copy)]
pub enum Carry {
    /// The host's layout IS the bench's: the bytes pass through.
    Same {
        /// `sizeof` at both.
        size: usize,
    },
    /// The layouts differ: carry by name ([`HostAbi::to_host`] / [`HostAbi::from_host`]).
    Carried {
        /// The bench layout (what the encoders write).
        bench: Resolved,
        /// The host's layout (what the ioctl must carry).
        host: Resolved,
    },
}

impl Carry {
    /// The size the host's frontend expects for this struct.
    #[must_use]
    pub fn host_size(&self) -> usize {
        match self {
            Carry::Same { size } => *size,
            Carry::Carried { host, .. } => host.size(),
        }
    }
}

/// Why a struct cannot cross to this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAbiError {
    /// The host driver's version is not one of the measured tags.
    Unmeasured {
        /// The host driver.
        version: DriverVersion,
    },
    /// The struct does not exist at the bench or at the host version.
    Layout(LayoutError),
    /// The bench body sets a field the host's layout does not have — the host cannot be asked it.
    Unexpressible {
        /// The C type.
        strukt: &'static str,
        /// The fields carrying data that the host's version lacks.
        fields: Vec<&'static str>,
        /// The host driver.
        version: DriverVersion,
    },
    /// The transcoder refused (an array the target cannot hold, a narrowing that loses data, …).
    Transcode {
        /// The C type.
        strukt: &'static str,
        /// Which direction: `true` = to the host.
        outgoing: bool,
        /// The transcoder's refusal.
        err: TranscodeError,
    },
    /// The body handed in is not the size its layout says.
    Size {
        /// The C type.
        strukt: &'static str,
        /// Bytes expected.
        want: usize,
        /// Bytes given.
        got: usize,
    },
}

impl fmt::Display for HostAbiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unmeasured { version } => write!(
                f,
                "host driver {version} is not a measured tag of the driver matrix; kayfabe carries \
                 host structs only between measured layouts (tools/drivermatrix/regen.sh {version})"
            ),
            Self::Layout(e) => write!(f, "{e}"),
            Self::Unexpressible { strukt, fields, version } => write!(
                f,
                "{strukt}: host driver {version} has no {fields:?}, and the request sets them — the host \
                 cannot be asked this; refusing rather than letting it read the bytes as another field"
            ),
            Self::Transcode { strukt, outgoing, err } => write!(
                f,
                "{strukt} could not be carried {} the host driver's layout: {err}",
                if *outgoing { "to" } else { "back from" }
            ),
            Self::Size { strukt, want, got } => write!(f, "{strukt}: body is {got} bytes, its layout is {want}"),
        }
    }
}

impl std::error::Error for HostAbiError {}

impl From<LayoutError> for HostAbiError {
    fn from(e: LayoutError) -> Self {
        Self::Layout(e)
    }
}

impl HostAbi {
    /// The measured ABI of this host driver.
    ///
    /// # Errors
    /// [`HostAbiError::Unmeasured`] for a version outside the committed sweep.
    pub fn for_host(host: HostDriverVersion) -> Result<Self, HostAbiError> {
        let version = DriverVersion { major: host.major, minor: host.minor, patch: host.patch };
        if !is_measured(version) {
            return Err(HostAbiError::Unmeasured { version });
        }
        Ok(Self { version })
    }

    /// The host driver version this ABI was resolved for.
    #[must_use]
    pub fn version(&self) -> DriverVersion {
        self.version
    }

    /// Is the host the bench driver itself (where the encoders' layouts were written)?
    #[must_use]
    pub fn is_bench(&self) -> bool {
        self.version == crate::versions::BENCH_DRIVER
    }

    /// How `runs` crosses to this host.
    ///
    /// # Errors
    /// [`HostAbiError::Layout`] when the struct is absent at the bench or at the host.
    pub fn carry(&self, runs: &'static StructRuns) -> Result<Carry, HostAbiError> {
        let bench = Resolved::of(runs, crate::versions::BENCH_DRIVER)?;
        let host = Resolved::of(runs, self.version)?;
        if std::ptr::eq(bench.layout, host.layout) {
            Ok(Carry::Same { size: bench.size() })
        } else {
            Ok(Carry::Carried { bench, host })
        }
    }

    /// The size the host's frontend expects for `runs`.
    ///
    /// # Errors
    /// As [`Self::carry`].
    pub fn host_size(&self, runs: &'static StructRuns) -> Result<usize, HostAbiError> {
        Ok(self.carry(runs)?.host_size())
    }

    /// A bench-layout body, carried to the host's layout (outgoing).
    ///
    /// # Errors
    /// [`HostAbiError::Unexpressible`] when a field the host lacks carries data;
    /// [`HostAbiError::Transcode`] / [`HostAbiError::Size`] / [`HostAbiError::Layout`].
    pub fn to_host(&self, runs: &'static StructRuns, bench_body: &[u8]) -> Result<Vec<u8>, HostAbiError> {
        match self.carry(runs)? {
            Carry::Same { size } => {
                if bench_body.len() != size {
                    return Err(HostAbiError::Size { strukt: runs.name, want: size, got: bench_body.len() });
                }
                Ok(bench_body.to_vec())
            }
            Carry::Carried { bench, host } => {
                if bench_body.len() != bench.size() {
                    return Err(HostAbiError::Size { strukt: runs.name, want: bench.size(), got: bench_body.len() });
                }
                let (out, dropped) = transcode(&bench, &host, bench_body, &[])
                    .map_err(|err| HostAbiError::Transcode { strukt: runs.name, outgoing: true, err })?;
                if !dropped.is_empty() {
                    return Err(HostAbiError::Unexpressible { strukt: runs.name, fields: dropped, version: self.version });
                }
                Ok(out)
            }
        }
    }

    /// A host-layout body (the host's reply), carried back to the bench layout (incoming). Fields
    /// only the host's version has are dropped — the bench encoders cannot read them.
    ///
    /// # Errors
    /// [`HostAbiError::Transcode`] (e.g. the host returned more array entries than the bench
    /// struct holds) / [`HostAbiError::Size`] / [`HostAbiError::Layout`].
    pub fn from_host(&self, runs: &'static StructRuns, host_body: &[u8]) -> Result<Vec<u8>, HostAbiError> {
        match self.carry(runs)? {
            Carry::Same { size } => {
                if host_body.len() != size {
                    return Err(HostAbiError::Size { strukt: runs.name, want: size, got: host_body.len() });
                }
                Ok(host_body.to_vec())
            }
            Carry::Carried { bench, host } => {
                if host_body.len() != host.size() {
                    return Err(HostAbiError::Size { strukt: runs.name, want: host.size(), got: host_body.len() });
                }
                transcode(&host, &bench, host_body, &[])
                    .map(|(b, _dropped)| b)
                    .map_err(|err| HostAbiError::Transcode { strukt: runs.name, outgoing: false, err })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::matrix as m;

    fn host(v: &str) -> HostAbi {
        HostAbi::for_host(HostDriverVersion::parse(v).expect("version")).expect("measured")
    }

    #[test]
    fn an_unmeasured_host_is_refused_by_name() {
        let r = HostAbi::for_host(HostDriverVersion::parse("580.142").expect("version"));
        assert!(matches!(r, Err(HostAbiError::Unmeasured { .. })), "{r:?}");
    }

    #[test]
    fn the_bench_host_passes_every_struct_through() {
        let h = host("580.159.04");
        assert!(h.is_bench());
        for runs in m::ALL_STRUCTS {
            if let Ok(c) = h.carry(runs) {
                assert!(matches!(c, Carry::Same { .. }), "{} is carried at the bench", runs.name);
            }
        }
    }

    /// `NVOS46` at 575: 56 bytes, `dmaOffset` at +40 where the bench has +48. A plain map carries;
    /// a kind override (a field 575 lacks) is refused by name, never sent as `dmaOffset`'s bytes.
    #[test]
    fn nvos46_carries_to_a_575_host_and_refuses_a_kind_override() {
        let h = host("575.57.08");
        let runs = &m::NVOS46_PARAMETERS;
        let b = Resolved::of(runs, crate::versions::BENCH_DRIVER).expect("bench");
        let mut body = vec![0u8; b.size()];
        let put = |body: &mut Vec<u8>, p: &'static str, v: u64, w: usize| {
            let o = b.need(p).expect(p).off();
            body[o..o + w].copy_from_slice(&v.to_le_bytes()[..w]);
        };
        put(&mut body, "hClient", 0xc1e0_0001, 4);
        put(&mut body, "hMemory", 0xcafe_0004, 4);
        put(&mut body, "offset", 0x20_0000, 8);
        put(&mut body, "length", 0x1000, 8);
        put(&mut body, "flags", 0x10, 4);
        put(&mut body, "dmaOffset", 0x7f00_0000, 8);
        assert_eq!(h.host_size(runs).expect("size"), 56);
        let out = h.to_host(runs, &body).expect("carried");
        assert_eq!(out.len(), 56);
        assert_eq!(u64::from_le_bytes(out[40..48].try_into().expect("8")), 0x7f00_0000, "dmaOffset at +40 on 575");
        let back = h.from_host(runs, &out).expect("back");
        assert_eq!(back, body, "a plain map round-trips");
        put(&mut body, "kindOverride", 0xfe, 4);
        match h.to_host(runs, &body) {
            Err(HostAbiError::Unexpressible { fields, .. }) => assert_eq!(fields, vec!["kindOverride"]),
            other => panic!("expected Unexpressible, got {other:?}"),
        }
    }

    /// `GPFIFO_SCHEDULE` at ≤570 has no `bSkipEnable`: asking for it is refused, a plain enable carries.
    #[test]
    fn gpfifo_schedule_skip_enable_is_unexpressible_below_575() {
        let h = host("570.148.08");
        let runs = &m::NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS;
        assert_eq!(h.host_size(runs).expect("size"), 2);
        assert_eq!(h.to_host(runs, &[1, 0, 0]).expect("enable"), vec![1, 0]);
        assert!(matches!(h.to_host(runs, &[1, 0, 1]), Err(HostAbiError::Unexpressible { .. })));
    }

    /// `GET_CLASSLIST_V2` holds 100 classes at 555–575 and 200 at the bench: a host reply carries
    /// back into the larger bench array.
    #[test]
    fn a_smaller_host_classlist_reply_carries_back() {
        let h = host("575.57.08");
        let runs = &m::NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS;
        let hs = h.host_size(runs).expect("size");
        assert_eq!(hs, 404);
        let mut reply = vec![0u8; hs];
        reply[0..4].copy_from_slice(&3u32.to_le_bytes());
        for (i, c) in [0xc56fu32, 0xc6b5, 0xc7c0].iter().enumerate() {
            reply[4 + i * 4..8 + i * 4].copy_from_slice(&c.to_le_bytes());
        }
        let back = h.from_host(runs, &reply).expect("back");
        assert_eq!(back.len(), 804);
        assert_eq!(&back[..16], &reply[..16]);
    }

    /// `NV_CHANNEL_ALLOC_PARAMS` at 610 inserts `hHandleVASpace`: every later field moves by 4 and
    /// the carry lands them where 610 reads them.
    #[test]
    fn channel_alloc_params_carry_to_610() {
        let h = host("610.57.04");
        let runs = &m::NV_CHANNEL_ALLOC_PARAMS;
        let b = Resolved::of(runs, crate::versions::BENCH_DRIVER).expect("bench");
        let t = Resolved::of(runs, h.version()).expect("610");
        let mut body = vec![0u8; b.size()];
        let e = b.need("engineType").expect("engineType").off();
        body[e..e + 4].copy_from_slice(&0x13u32.to_le_bytes());
        let out = h.to_host(runs, &body).expect("carried");
        let te = t.need("engineType").expect("engineType").off();
        assert_ne!(e, te, "engineType moved at 610");
        assert_eq!(u32::from_le_bytes(out[te..te + 4].try_into().expect("4")), 0x13);
    }
}
