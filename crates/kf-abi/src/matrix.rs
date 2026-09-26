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
    /// For an ARRAY: its element size in bytes (`-1` = multi-dimensional). `0` = not an array.
    /// Measured from the DWARF element type — what [`transcode`] resizes an array by.
    pub elem: i32,
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

    /// A 1-D array's `(element bytes, count)`; `None` for a non-array or a multi-dim array.
    #[must_use]
    pub fn array(self) -> Option<(usize, usize)> {
        let e = usize::try_from(self.elem).ok().filter(|e| *e > 0)?;
        Some((e, self.bytes()? / e))
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

    /// ★ The value of a constant the sweep measured IDENTICAL and PRESENT at every tag (exactly
    /// one run, which is contiguous over all of `MEASURED`), usable in a `const` — so a future
    /// sweep in which it varies or vanishes is a BUILD failure at the use site, never a stale id
    /// answered to the version that moved it. A constant that does vary is read per version
    /// with [`ValueRuns::at_u32`].
    #[must_use]
    pub const fn everywhere_u32(&self) -> u32 {
        assert!(
            self.runs.len() == 1,
            "the measured value varies across tags: read it per version (at_u32)"
        );
        match self.runs[0].value {
            Some(x) if x <= u32::MAX as u64 => x as u32,
            _ => panic!("the constant is absent (or wider than 32 bits) at the measured tags"),
        }
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

// =====================================================================================
// ★★★ The measured TRANSCODER — one encoder, every version's layout
// =====================================================================================

/// Why a body could not be carried into another version's layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscodeError {
    /// An array shrinks at the target version and a dropped element carries data. For a
    /// per-GPC / per-engine array this means the hardware has more units than the guest's
    /// struct can describe — a refusal, never a silent loss. Index-keyed lists the guest
    /// cannot name beyond its capacity are declared truncatable by the caller.
    Truncates {
        /// The array's path.
        path: &'static str,
        /// The first dropped element that is non-zero.
        index: usize,
    },
    /// A scalar narrows at the target version and the value does not fit.
    Narrows {
        /// The field's path.
        path: &'static str,
    },
    /// A shape this transcoder does not carry (bitfields, a multi-dimensional array that
    /// changes size, an element whose kind differs between the versions).
    Unsupported {
        /// The field's path.
        path: &'static str,
    },
    /// The body is shorter than the source layout.
    Short {
        /// Bytes needed.
        need: usize,
        /// Bytes given.
        got: usize,
    },
}

impl fmt::Display for TranscodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncates { path, index } => {
                write!(f, "{path}[{index}] carries data the target version's struct has no room for")
            }
            Self::Narrows { path } => write!(f, "{path} does not fit the target version's narrower field"),
            Self::Unsupported { path } => write!(f, "{path} has a shape the transcoder does not carry"),
            Self::Short { need, got } => write!(f, "body is {got} bytes, the source layout is {need}"),
        }
    }
}

impl std::error::Error for TranscodeError {}

fn parent_of(path: &str) -> &str {
    path.rfind('.').map_or("", |i| &path[..i])
}

fn name_of(path: &str) -> &str {
    path.rfind('.').map_or(path, |i| &path[i + 1..])
}

/// The direct children of container `parent` (`""` = the root): field paths whose parent is it,
/// excluding array-element paths (`x[]`, which belong to their array `x`).
fn children<'a>(l: &'a Layout, parent: &str) -> Vec<(&'static str, FieldAt)> {
    l.fields
        .iter()
        .filter(|(p, _)| *p != "." && !p.ends_with("[]") && parent_of(p) == parent)
        .map(|(p, f)| (*p, *f))
        .collect::<Vec<_>>()
}

fn has_children(l: &Layout, path: &str) -> bool {
    l.fields.iter().any(|(p, _)| *p != "." && parent_of(p) == path)
}

/// Children overlap ⇒ the container is a union: carried as raw bytes.
fn is_union(l: &Layout, path: &str) -> bool {
    let mut offs: Vec<(u32, i32)> = children(l, path).iter().map(|(_, f)| (f.off, f.size)).collect();
    offs.sort_unstable();
    offs.windows(2).any(|w| (w[0].0 as i64) + (w[0].1.max(0) as i64) > w[1].0 as i64)
}

/// ★★★ Carry `body`, encoded at the `from` layout, into the `to` layout **by field name**
/// (`docs/design/V3_DRIVER_MATRIX.md` §4.5).
///
/// Every field present in both layouts is copied from its source offset to its target offset;
/// structs recurse; arrays copy `min(count)` elements (struct elements recurse, scalar elements
/// copy — zero-extending a widened element, refusing a narrowed one that does not fit); a union
/// is carried as raw bytes. A field only the TARGET has stays zero (the guest's own newer field
/// that the source version never stated). A field only the SOURCE has is dropped (the guest's
/// version does not have it) and reported ONCE in the returned list when it carried data.
///
/// ⊘ An array that SHRINKS refuses when a dropped element carries data, unless its path is in
/// `truncatable` — the caller's statement that the list is index-keyed and the guest cannot name
/// entries beyond its own capacity. The default is the safe direction.
///
/// # Errors
/// [`TranscodeError`] by path.
pub fn transcode(
    from: &Resolved,
    to: &Resolved,
    body: &[u8],
    truncatable: &[&str],
) -> Result<(Vec<u8>, Vec<&'static str>), TranscodeError> {
    if body.len() < from.size() {
        return Err(TranscodeError::Short { need: from.size(), got: body.len() });
    }
    let mut out = vec![0u8; to.size()];
    let mut dropped = Vec::new();
    // (path, from abs base of this instance, from elem-0 base, to abs base, to elem-0 base)
    carry(from.layout, to.layout, "", 0, 0, 0, 0, body, &mut out, truncatable, &mut dropped)?;
    Ok((out, dropped))
}

#[allow(clippy::too_many_arguments)]
fn carry(
    fl: &Layout,
    tl: &Layout,
    container: &str,
    f_base: usize,
    f_e0: usize,
    t_base: usize,
    t_e0: usize,
    body: &[u8],
    out: &mut [u8],
    truncatable: &[&str],
    dropped: &mut Vec<&'static str>,
) -> Result<(), TranscodeError> {
    let fkids = children(fl, container);
    let tkids = children(tl, container);
    for (fp, ff) in &fkids {
        let name = name_of(fp);
        let Some((tp, tf)) = tkids.iter().find(|(p, _)| name_of(p) == name) else {
            let at = f_base + (ff.off() - f_e0);
            if let Some(b) = ff.bytes()
                && body[at..at + b].iter().any(|x| *x != 0)
                && !dropped.contains(fp)
            {
                dropped.push(*fp);
            }
            continue;
        };
        if ff.size < 0 || tf.size < 0 {
            if ff != tf {
                return Err(TranscodeError::Unsupported { path: fp });
            }
            // A bitfield at the same place in both: its bytes are shared with its neighbours
            // and copied with them below (the enclosing scalar storage is identical).
            continue;
        }
        let fa = f_base + (ff.off() - f_e0);
        let ta = t_base + (tf.off() - t_e0);
        match (ff.array(), tf.array()) {
            (Some((fe, fnn)), Some((te, tn))) => {
                let el_path_f = format!("{fp}[]");
                let struct_elems = fl.fields.iter().any(|(p, _)| *p == el_path_f.as_str()) && has_children(fl, &el_path_f);
                for i in 0..fnn.min(tn) {
                    if struct_elems {
                        let fe0 = fl.fields.iter().find(|(p, _)| *p == el_path_f.as_str()).map(|(_, f)| f.off()).unwrap_or(ff.off());
                        let el_path_t = format!("{tp}[]");
                        let te0 = tl.fields.iter().find(|(p, _)| *p == el_path_t.as_str()).map(|(_, f)| f.off()).unwrap_or(tf.off());
                        let path: &'static str = fl.fields.iter().find(|(p, _)| *p == el_path_f.as_str()).map(|(p, _)| *p).unwrap_or(fp);
                        carry(fl, tl, path, fa + i * fe, fe0, ta + i * te, te0, body, out, truncatable, dropped)?;
                    } else {
                        copy_scalar(&body[fa + i * fe..fa + (i + 1) * fe], &mut out[ta + i * te..ta + (i + 1) * te], fp)?;
                    }
                }
                if fnn > tn {
                    for i in tn..fnn {
                        if body[fa + i * fe..fa + (i + 1) * fe].iter().any(|x| *x != 0) {
                            if truncatable.contains(fp) {
                                break;
                            }
                            return Err(TranscodeError::Truncates { path: fp, index: i });
                        }
                    }
                }
            }
            (None, None) if has_children(fl, fp) && has_children(tl, tp) && !is_union(fl, fp) && !is_union(tl, tp) => {
                carry(fl, tl, fp, f_base, f_e0, t_base, t_e0, body, out, truncatable, dropped)?;
            }
            (None, None) if has_children(fl, fp) || has_children(tl, tp) => {
                // A union (or an aggregate on one side only): raw bytes, refusing a tail the
                // target cannot hold.
                let (fb, tb) = (ff.bytes().unwrap_or(0), tf.bytes().unwrap_or(0));
                let n = fb.min(tb);
                out[ta..ta + n].copy_from_slice(&body[fa..fa + n]);
                if fb > tb && body[fa + tb..fa + fb].iter().any(|x| *x != 0) {
                    return Err(TranscodeError::Truncates { path: fp, index: tb });
                }
            }
            (None, None) => {
                let (fb, tb) = (ff.bytes().unwrap_or(0), tf.bytes().unwrap_or(0));
                copy_scalar(&body[fa..fa + fb], &mut out[ta..ta + tb], fp)?;
            }
            _ => return Err(TranscodeError::Unsupported { path: fp }),
        }
    }
    Ok(())
}

/// Copy a little-endian scalar between widths: zero-extend when widening, refuse a value that
/// does not fit when narrowing.
fn copy_scalar(src: &[u8], dst: &mut [u8], path: &'static str) -> Result<(), TranscodeError> {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    if src.len() > dst.len() && src[dst.len()..].iter().any(|x| *x != 0) {
        return Err(TranscodeError::Narrows { path });
    }
    Ok(())
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

    // ── the transcoder, on synthetic layouts (independent of the generated data) ──────────

    const fn fa(off: u32, size: i32, elem: i32) -> FieldAt {
        FieldAt { off, size, elem }
    }

    /// "580-like": a count, a per-GPC mask array of 16 u32, and 4 entries of a struct
    /// {id u32, phys u32 (580-only), val u16}.
    static FROM: Layout = Layout {
        size: 4 + 64 + 4 * 12,
        fields: &[
            ("count", fa(0, 4, 0)),
            ("mask", fa(4, 64, 4)),
            ("ent", fa(68, 48, 12)),
            ("ent[]", fa(68, 12, 0)),
            ("ent[].id", fa(68, 4, 0)),
            ("ent[].phys", fa(72, 4, 0)),
            ("ent[].val", fa(76, 2, 0)),
        ],
    };
    /// "575-like": count, 12 masks, 3 entries of {id u32, val u32 (widened)}, and a 575-only tail.
    static TO: Layout = Layout {
        size: 4 + 48 + 3 * 8 + 4,
        fields: &[
            ("count", fa(0, 4, 0)),
            ("mask", fa(4, 48, 4)),
            ("ent", fa(52, 24, 8)),
            ("ent[]", fa(52, 8, 0)),
            ("ent[].id", fa(52, 4, 0)),
            ("ent[].val", fa(56, 4, 0)),
            ("tail", fa(76, 4, 0)),
        ],
    };

    fn res(l: &'static Layout) -> Resolved {
        Resolved { layout: l, strukt: "S", version: DriverVersion { major: 1, minor: 0, patch: 0 } }
    }

    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().expect("4"))
    }

    #[test]
    fn transcode_carries_fields_by_name_across_layouts() {
        let mut body = vec![0u8; FROM.size()];
        body[0..4].copy_from_slice(&3u32.to_le_bytes());
        for i in 0..12 {
            body[4 + 4 * i..8 + 4 * i].copy_from_slice(&(0x100 + i as u32).to_le_bytes());
        }
        for i in 0..3 {
            let o = 68 + 12 * i;
            body[o..o + 4].copy_from_slice(&(10 + i as u32).to_le_bytes());
            body[o + 4..o + 8].copy_from_slice(&0xdead_u32.to_le_bytes()); // phys: 580-only
            body[o + 8..o + 10].copy_from_slice(&(0x7000 + i as u16).to_le_bytes());
        }
        let (out, dropped) = transcode(&res(&FROM), &res(&TO), &body, &[]).expect("carries");
        assert_eq!(out.len(), TO.size());
        assert_eq!(u32_at(&out, 0), 3);
        for i in 0..12 {
            assert_eq!(u32_at(&out, 4 + 4 * i), 0x100 + i as u32, "mask[{i}]");
        }
        for i in 0..3 {
            let o = 52 + 8 * i;
            assert_eq!(u32_at(&out, o), 10 + i as u32, "ent[{i}].id");
            assert_eq!(u32_at(&out, o + 4), 0x7000 + i as u32, "ent[{i}].val widened u16 -> u32");
        }
        assert_eq!(u32_at(&out, 76), 0, "a target-only field stays zero");
        assert_eq!(dropped, vec!["ent[].phys"], "the source-only field with data is reported");
    }

    #[test]
    fn a_shrinking_array_refuses_a_dropped_element_with_data_unless_declared_truncatable() {
        let mut body = vec![0u8; FROM.size()];
        body[4 + 4 * 13..8 + 4 * 13].copy_from_slice(&1u32.to_le_bytes()); // mask[13]: a 14th GPC
        assert_eq!(
            transcode(&res(&FROM), &res(&TO), &body, &[]).map(|_| ()),
            Err(TranscodeError::Truncates { path: "mask", index: 13 })
        );
        assert!(transcode(&res(&FROM), &res(&TO), &body, &["mask"]).is_ok(), "declared truncatable");
        // An all-zero tail is not data: dropping it loses nothing.
        let zero = vec![0u8; FROM.size()];
        assert!(transcode(&res(&FROM), &res(&TO), &zero, &[]).is_ok());
    }

    #[test]
    fn a_narrowed_scalar_that_does_not_fit_is_refused() {
        // TO → FROM direction: ent[].val narrows u32 -> u16.
        let mut body = vec![0u8; TO.size()];
        body[56..60].copy_from_slice(&0x1_0000u32.to_le_bytes());
        assert_eq!(
            transcode(&res(&TO), &res(&FROM), &body, &[]).map(|_| ()),
            Err(TranscodeError::Narrows { path: "ent[].val" })
        );
        body[56..60].copy_from_slice(&0xffffu32.to_le_bytes());
        let (out, _) = transcode(&res(&TO), &res(&FROM), &body, &[]).expect("fits");
        assert_eq!(u16::from_le_bytes([out[76], out[77]]), 0xffff);
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
