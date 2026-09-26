//! ★★★ The **driver matrix** — NVIDIA ABI facts MEASURED per driver version, and the one lookup
//! rule every consumer goes through (`docs/design/V3_DRIVER_MATRIX.md` §3–§4).
//!
//! # What is in the generated half
//!
//! [`crate::generated::matrix`] is produced by `tools/drivermatrix/` from the open kernel modules:
//! every ogkm tag in [`MEASURED`](crate::generated::matrix::MEASURED) was compiled, struct layouts
//! were read out of the DWARF `gcc` emitted under the tag's OWN `src/nvidia/Makefile` flags, and
//! constants came from the preprocessor's macro table plus a compiled `printf`. Nothing in it was
//! typed by hand, transcribed from a neighbouring tag, or captured from a running driver.
//!
//! # The lookup rule, and why it has no nearest-neighbour arm
//!
//! A version is either **measured** — it is one of the tags in `MEASURED`, and its value is the
//! one its run records (possibly *absent*) — or it is [`Unmeasured`], and every lookup refuses it
//! by name. ⊘ Deliberately NOT "the newest measured tag ≤ the version": the ogkm history moves
//! fields *inside* a branch (`GspSystemInfo` gained a field at 580.95.05 and another at
//! 580.105.08; `g_rpc-structures.h` changed at 580.126.09, 580.159.04 and 580.173.02), so a
//! release between two measured tags is exactly where a borrowed layout is silently wrong. The
//! way past an [`Unmeasured`] is one command — `tools/drivermatrix/regen.sh <tag>` — not a guess.
//!
//! # Absent is an answer, not an error
//!
//! A field that a version does not have is [`None`] from [`Layout::field`], and the consumer
//! decides what that means (skip a write, refuse a feature by name). A version that was never
//! measured is an error. The two are different findings and the types keep them apart.

use crate::DriverVersion;
use std::fmt;

/// Where one field sits in one measured layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldAt {
    /// Byte offset from the start of the struct.
    pub off: u32,
    /// Size in bytes. ⚠ A **bitfield** is recorded as a negative bit count (the sweep's
    /// convention); [`FieldAt::bytes`] refuses it rather than inventing a byte width.
    pub size: i32,
}

impl FieldAt {
    /// The offset as a `usize`.
    #[must_use]
    pub fn off(self) -> usize {
        self.off as usize
    }

    /// The width in bytes, or `None` for a bitfield.
    #[must_use]
    pub fn bytes(self) -> Option<usize> {
        usize::try_from(self.size).ok()
    }

    /// `off .. off + bytes` — `None` for a bitfield.
    #[must_use]
    pub fn range(self) -> Option<core::ops::Range<usize>> {
        self.bytes().map(|b| self.off()..self.off() + b)
    }
}

/// One distinct layout of a struct, restricted to the fields kayfabe consumes.
#[derive(Debug, PartialEq, Eq)]
pub struct Layout {
    /// `sizeof` at this layout.
    pub size: u32,
    /// `(path, where)` for every consumed field PRESENT in this layout. Paths use `.` for members
    /// and `[]` for "element 0 of an array" (the stride is the array element's own size).
    pub fields: &'static [(&'static str, FieldAt)],
}

impl Layout {
    /// A consumed field, or `None` if this layout does not have it.
    #[must_use]
    pub fn field(&self, path: &str) -> Option<FieldAt> {
        self.fields.iter().find(|(p, _)| *p == path).map(|(_, f)| *f)
    }

    /// `sizeof` as a `usize`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.size as usize
    }
}

/// One run of consecutive measured tags over which an item is identical.
#[derive(Debug)]
pub struct Run<T: 'static> {
    /// The first measured tag of the run.
    pub first: DriverVersion,
    /// The last measured tag of the run (inclusive).
    pub last: DriverVersion,
    /// The item's value over the run; `None` = the item does not exist at these tags.
    pub value: Option<T>,
}

/// A struct's consumed layouts over the measured tags.
#[derive(Debug)]
pub struct StructRuns {
    /// The C type name.
    pub name: &'static str,
    /// Ascending, contiguous over [`MEASURED`](crate::generated::matrix::MEASURED).
    pub runs: &'static [Run<&'static Layout>],
}

/// A constant's value over the measured tags.
#[derive(Debug)]
pub struct ValueRuns {
    /// `entry:NAME` as the spec named it.
    pub name: &'static str,
    /// Ascending, contiguous over [`MEASURED`](crate::generated::matrix::MEASURED).
    pub runs: &'static [Run<u64>],
}

/// The refusal: this driver version was never measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmeasured {
    /// The version asked about.
    pub version: DriverVersion,
    /// The item being looked up (a struct or constant name), for the log line.
    pub item: &'static str,
}

impl fmt::Display for Unmeasured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.version;
        write!(
            f,
            "driver {}.{}.{:02} was never measured (looking up {}); kayfabe answers only the \
             ogkm tags in the committed driver matrix, and a release between two measured tags \
             can move a field — measure it with `tools/drivermatrix/regen.sh {}.{}.{:02}` \
             instead of borrowing a neighbour's layout",
            v.major, v.minor, v.patch, self.item, v.major, v.minor, v.patch
        )
    }
}

impl std::error::Error for Unmeasured {}

/// Is `version` one of the measured tags?
#[must_use]
pub fn is_measured(version: DriverVersion) -> bool {
    crate::generated::matrix::MEASURED.binary_search(&version).is_ok()
}

fn find<T: Copy>(
    runs: &'static [Run<T>],
    item: &'static str,
    version: DriverVersion,
) -> Result<Option<T>, Unmeasured> {
    if !is_measured(version) {
        return Err(Unmeasured { version, item });
    }
    // Runs are contiguous over MEASURED, so the run containing a measured version is the one
    // whose [first, last] brackets it.
    runs.iter()
        .find(|r| r.first <= version && version <= r.last)
        .map(|r| r.value)
        .ok_or(Unmeasured { version, item })
}

impl StructRuns {
    /// This struct's consumed layout at `version` — `Ok(None)` where the struct does not exist.
    ///
    /// # Errors
    /// [`Unmeasured`] for a version outside the committed sweep.
    pub fn at(&'static self, version: DriverVersion) -> Result<Option<&'static Layout>, Unmeasured> {
        find(self.runs, self.name, version)
    }
}

impl ValueRuns {
    /// This constant at `version` — `Ok(None)` where it does not exist.
    ///
    /// # Errors
    /// [`Unmeasured`] for a version outside the committed sweep.
    pub fn at(&'static self, version: DriverVersion) -> Result<Option<u64>, Unmeasured> {
        find(self.runs, self.name, version)
    }

    /// The same, narrowed to `u32` (every RPC number, control id and class id is one).
    ///
    /// # Errors
    /// [`Unmeasured`] for a version outside the committed sweep; a value wider than 32 bits is
    /// reported as absent, never truncated.
    pub fn at_u32(&'static self, version: DriverVersion) -> Result<Option<u32>, Unmeasured> {
        Ok(self.at(version)?.and_then(|v| u32::try_from(v).ok()))
    }
}

/// A consumed field that a measured layout does not have — the unit of the per-version gap
/// report (`V3_DRIVER_MATRIX.md` §6): every one of these is a feature the guest or host at that
/// version expects differently from the one the encoder was written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingField {
    /// The C type.
    pub strukt: &'static str,
    /// The field path.
    pub path: &'static str,
    /// The version.
    pub version: DriverVersion,
}

impl fmt::Display for MissingField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.version;
        write!(
            f,
            "{}.{} does not exist at driver {}.{}.{:02} (measured); the consumer has no \
             encoding for that layout",
            self.strukt, self.path, v.major, v.minor, v.patch
        )
    }
}

impl std::error::Error for MissingField {}

/// Either refusal a layout resolution can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// The version was never measured.
    Unmeasured(Unmeasured),
    /// The struct does not exist at this version.
    NoStruct {
        /// The C type.
        strukt: &'static str,
        /// The version.
        version: DriverVersion,
    },
    /// A field the consumer needs does not exist at this version.
    Missing(MissingField),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unmeasured(u) => u.fmt(f),
            Self::NoStruct { strukt, version: v } => write!(
                f,
                "{strukt} does not exist at driver {}.{}.{:02} (measured)",
                v.major, v.minor, v.patch
            ),
            Self::Missing(m) => m.fmt(f),
        }
    }
}

impl std::error::Error for LayoutError {}

impl From<Unmeasured> for LayoutError {
    fn from(u: Unmeasured) -> Self {
        Self::Unmeasured(u)
    }
}

/// A resolved layout plus the version it was resolved for, so a missing field can be named.
#[derive(Debug, Clone, Copy)]
pub struct Resolved {
    /// The layout.
    pub layout: &'static Layout,
    /// The C type.
    pub strukt: &'static str,
    /// The version.
    pub version: DriverVersion,
}

impl Resolved {
    /// Resolve `runs` at `version`, refusing an unmeasured version and an absent struct.
    ///
    /// # Errors
    /// [`LayoutError::Unmeasured`] / [`LayoutError::NoStruct`].
    pub fn of(runs: &'static StructRuns, version: DriverVersion) -> Result<Self, LayoutError> {
        let layout = runs.at(version)?.ok_or(LayoutError::NoStruct {
            strukt: runs.name,
            version,
        })?;
        Ok(Self {
            layout,
            strukt: runs.name,
            version,
        })
    }

    /// A field the consumer REQUIRES.
    ///
    /// # Errors
    /// [`LayoutError::Missing`] naming the struct, the path and the version.
    pub fn need(&self, path: &'static str) -> Result<FieldAt, LayoutError> {
        self.layout.field(path).ok_or(LayoutError::Missing(MissingField {
            strukt: self.strukt,
            path,
            version: self.version,
        }))
    }

    /// A field the consumer can do without (absent ⇒ skip).
    #[must_use]
    pub fn maybe(&self, path: &str) -> Option<FieldAt> {
        self.layout.field(path)
    }

    /// `sizeof`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.layout.size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::matrix::{ALL_STRUCTS, ALL_VALUES, MEASURED};

    /// The measured list is strictly ascending, so `binary_search` is a valid membership test.
    #[test]
    fn measured_is_strictly_ascending() {
        assert!(MEASURED.windows(2).all(|w| w[0] < w[1]), "MEASURED must be sorted");
        assert!(!MEASURED.is_empty());
    }

    /// Every generated table covers exactly the measured tags, contiguously — so a lookup at a
    /// measured version always lands in exactly one run, and an off-by-one in the generator
    /// (a gap or an overlap) is a test failure, not a wrong answer.
    #[test]
    fn every_table_tiles_the_measured_tags() {
        fn tiles<T>(name: &str, runs: &[Run<T>]) {
            let mut i = 0usize;
            for r in runs {
                assert_eq!(r.first, MEASURED[i], "{name}: run starts off the measured grid");
                let j = MEASURED.iter().position(|m| *m == r.last).expect("run ends on a tag");
                assert!(j >= i, "{name}: run ends before it starts");
                i = j + 1;
            }
            assert_eq!(i, MEASURED.len(), "{name}: runs do not reach the last measured tag");
        }
        for s in ALL_STRUCTS {
            tiles(s.name, s.runs);
        }
        for v in ALL_VALUES {
            tiles(v.name, v.runs);
        }
    }

    /// ⊘ An unmeasured version is refused — including one strictly BETWEEN two measured tags,
    /// which is the nearest-neighbour case this module exists to forbid.
    #[test]
    fn a_version_between_measured_tags_is_refused_by_name() {
        let between = DriverVersion {
            major: 580,
            minor: 159,
            patch: 3,
        };
        assert!(!is_measured(between));
        let s = ALL_STRUCTS[0];
        let e = s.at(between).expect_err("unmeasured must refuse");
        assert_eq!(e.version, between);
        assert!(e.to_string().contains("580.159.03"), "{e}");
        assert!(e.to_string().contains("regen.sh"), "says how to measure it: {e}");
    }
}
