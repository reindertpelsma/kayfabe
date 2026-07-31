//! # `memtype` — the **effective** memory type, read back from the kernel rather than assumed
//!
//! ## ★★★ Why this module exists, and what it is not
//!
//! [`crate::CachePolicy`] is a **request**. It is checked structurally
//! ([`crate::cache::require_attainable`]): a backing kind implies an attainable policy, and a
//! request that disagrees is refused before the `mmap`. That check is necessary and it is
//! **not sufficient**, because the failure it exists to catch is a *silent downgrade that
//! happens inside the kernel after the request was accepted*:
//!
//! > x86's `reserve_pfn_range()` rewrites a write-back request to `UC-` for any physical
//! > range `/proc/iomem` does not call *System RAM*. No error. No log. The `mmap` returns
//! > success and the caller holds a pointer that is one to two orders of magnitude slower
//! > than the one it asked for.
//!
//! [`CachePolicy::WriteBack`]'s own rustdoc names that mechanism and names its symptom. What
//! did not exist until this module is a way to **find out whether it happened**. A request
//! and an outcome are different facts, and the whole cost of that defect class comes from
//! reading the first as though it were the second.
//!
//! ⊘ **This module installs nothing and changes nothing.** It has no way to make a mapping
//! write-back; `mmap(2)` has no cacheability argument and this module does not pretend to
//! supply one. It is an *instrument*: it reports what the kernel did.
//!
//! ## The three instruments, and what each one can and cannot see
//!
//! | instrument | what it reads | sees a downgrade? | limitation |
//! |---|---|---|---|
//! | [`classify_physical`] | `/proc/iomem` | **predicts** it | it is the kernel's own predicate, evaluated before the mapping exists |
//! | [`recorded_memtype`] | `/sys/kernel/debug/x86/pat_memtype_list` | **reports** it | needs `debugfs` and x86; a range with no entry is untracked, not write-back-by-proof |
//! | [`BandwidthWitness`] | the mapping itself, timed | **exhibits** it | needs a live mapping and a reference; a ratio, never a name |
//!
//! ★★ They are deliberately three, because each one alone has a way of being green while
//! wrong. `/proc/iomem` is a prediction about a mapping that does not exist yet. The PAT list
//! records *reservations* — a range with no entry is a range nobody tracked, which is the
//! normal state of ordinary memory and is therefore indistinguishable, from that file alone,
//! from a range nobody mapped. And a bandwidth ratio cannot tell `UC-` from `WC` on a read
//! path, because both are uncached for loads. Used together they pin the answer; used
//! singly, each is a claim.
//!
//! ## ★★ What this module can NOT tell you, stated so nobody reads it as more
//!
//! Every instrument here observes **decider 1**, the host userspace PTE. A guest access to a
//! memslot backed by this mapping is governed by the *combination* of decider 1, the
//! host's second-dimension page tables (EPT/NPT) and the guest's own PTE. On Intel, EPT sets
//! `IPAT` for a normal-RAM backing and the guest PTE is **ignored**; AMD NPT has no `IPAT`
//! and honours the guest PTE. So a host mapping that this module reports as write-back can
//! still be uncached *in the guest*, on one vendor and not the other — which is the exact
//! asymmetry that let the C artifact's `#111` survive a full green run on Intel hardware.
//!
//! ⊘ Nothing in userspace can read a guest's effective type. A consumer that needs that
//! answer must measure it **in the guest**, and this module's job is to stop the *host* half
//! from being assumed.

use std::fmt;
use std::fs;
use std::time::Instant;

use crate::CachePolicy;

/// Where the kernel records what it did.
const PAT_LIST: &str = "/sys/kernel/debug/x86/pat_memtype_list";
/// Where the kernel records what a physical range **is** — the predicate the downgrade
/// branches on.
const IOMEM: &str = "/proc/iomem";
/// The label `/proc/iomem` gives ordinary memory. The downgrade applies to everything else.
const SYSTEM_RAM: &str = "System RAM";

/// Why an instrument could not answer.
///
/// ⊘ Every variant means *"no answer"*, never *"the answer is fine"*. A caller that maps
/// [`MemtypeError::Unavailable`] to "assume write-back" has rebuilt the defect this module
/// exists to detect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemtypeError {
    /// The file the instrument reads is absent or unreadable — no `debugfs`, not x86, or
    /// insufficient privilege. Carries the path so the diagnostic names the missing thing.
    Unavailable {
        /// The file that could not be read.
        path: &'static str,
        /// The kernel's own complaint.
        why: String,
    },
    /// A line was present but not in the shape this parser knows. Loud rather than skipped:
    /// a format drift that silently produced *"no entry"* would read as *"untracked, so
    /// write-back"* and be exactly wrong.
    Unparsable {
        /// The file.
        path: &'static str,
        /// The line, trimmed.
        line: String,
    },
    /// ★★★ **The downgrade, caught.** The requested policy and the one the kernel actually
    /// installed are different.
    Downgraded {
        /// What the caller asked for.
        requested: CachePolicy,
        /// What the kernel recorded.
        effective: CachePolicy,
        /// Which instrument said so, and what it read.
        evidence: String,
    },
}

impl fmt::Display for MemtypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { path, why } => {
                write!(
                    f,
                    "the effective memory type is unreadable here: {path} ({why})"
                )
            }
            Self::Unparsable { path, line } => {
                write!(
                    f,
                    "{path} carried a line this parser does not know: {line:?}"
                )
            }
            Self::Downgraded {
                requested,
                effective,
                evidence,
            } => write!(
                f,
                "asked for {requested} and the kernel installed {effective} — {evidence}"
            ),
        }
    }
}

impl std::error::Error for MemtypeError {}

// ─────────────────────────────────────────────────────────────────────────────────────
// The kernel's own vocabulary
// ─────────────────────────────────────────────────────────────────────────────────────

/// A memory type **as the kernel spells it in its own PAT list**.
///
/// ★ It is a wider vocabulary than [`CachePolicy`], on purpose. `CachePolicy` is the set of
/// things a caller may *ask* for; this is the set of things the kernel may *answer*, and the
/// two differ in the one place that matters: [`Self::UncachedMinus`]. The kernel never writes
/// `uncached` where it means `uncached-minus`, and collapsing them at parse time would erase
/// the distinction that makes an x86 "uncached" mapping's real behaviour MTRR-dependent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KernelMemtype {
    /// `write-back` — cached.
    WriteBack,
    /// `write-through`.
    WriteThrough,
    /// `write-combining` — uncached, stores coalesce.
    WriteCombining,
    /// `write-protected`.
    WriteProtected,
    /// ★★ `uncached-minus` — the **weak** form, and the one a downgrade produces. It
    /// combines with MTRRs, so its real behaviour depends on state no process can read.
    /// This is the word `#111` is spelled in.
    UncachedMinus,
    /// `uncached` — the strict form.
    Uncached,
}

impl KernelMemtype {
    /// The kernel's own spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WriteBack => "write-back",
            Self::WriteThrough => "write-through",
            Self::WriteCombining => "write-combining",
            Self::WriteProtected => "write-protected",
            Self::UncachedMinus => "uncached-minus",
            Self::Uncached => "uncached",
        }
    }

    /// Parse the kernel's spelling. `None` for anything this parser does not know — the
    /// caller turns that into [`MemtypeError::Unparsable`] rather than into silence.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "write-back" => Self::WriteBack,
            "write-through" => Self::WriteThrough,
            "write-combining" => Self::WriteCombining,
            "write-protected" => Self::WriteProtected,
            "uncached-minus" => Self::UncachedMinus,
            "uncached" => Self::Uncached,
            _ => return None,
        })
    }

    /// The [`CachePolicy`] a caller would have had to request to get this.
    ///
    /// ★★ Both uncached forms map to [`CachePolicy::Uncached`], and `write-through` and
    /// `write-protected` map to [`CachePolicy::WriteBack`] because both are *cached* — the
    /// coarsening is deliberate and it is one-way. Nothing may go the other direction: the
    /// point of [`KernelMemtype`] existing is that the kernel's answer is finer than the
    /// caller's question, so the fine value is what gets carried in a diagnostic.
    #[must_use]
    pub const fn as_cache_policy(self) -> CachePolicy {
        match self {
            Self::WriteBack | Self::WriteThrough | Self::WriteProtected => CachePolicy::WriteBack,
            Self::WriteCombining => CachePolicy::WriteCombining,
            Self::UncachedMinus | Self::Uncached => CachePolicy::Uncached,
        }
    }
}

impl fmt::Display for KernelMemtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Instrument 1 — /proc/iomem, the predicate the downgrade branches on
// ─────────────────────────────────────────────────────────────────────────────────────

/// What `/proc/iomem` calls a physical range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysClass {
    /// ★★★ Whether **every byte** of the queried range is inside a region labelled
    /// `System RAM`. This is the predicate, and it is `all`, not `any`: a range that is
    /// half RAM is a range the downgrade applies to.
    pub system_ram: bool,
    /// The most specific label found covering the range's first byte, for the diagnostic.
    /// Empty when nothing covers it.
    pub label: String,
}

impl PhysClass {
    /// The policy a write-back request will actually get for this range, as far as this
    /// instrument can tell.
    ///
    /// ★ `system_ram == false` does **not** guarantee a downgrade — it guarantees the
    /// downgrade *applies*, and the range may already be reserved as something else by a
    /// driver that got there first. That is why this returns the pessimistic answer and
    /// [`recorded_memtype`] is consulted for the actual one.
    #[must_use]
    pub const fn write_back_attainable(&self) -> bool {
        self.system_ram
    }
}

/// ★★★ **Classify a host-physical range the way the kernel's own downgrade does.**
///
/// Parses `/proc/iomem`, whose lines are `  <start>-<end> : <label>` with leading spaces
/// encoding nesting depth. The deepest label covering the range's first byte wins, which is
/// how a driver's claim (`nvidia`) is reported rather than the bridge that contains it.
///
/// # Errors
/// [`MemtypeError::Unavailable`] if `/proc/iomem` cannot be read;
/// [`MemtypeError::Unparsable`] on a line whose shape this parser does not know.
///
/// # Panics
/// Never.
pub fn classify_physical(base: u64, len: u64) -> Result<PhysClass, MemtypeError> {
    let text = fs::read_to_string(IOMEM).map_err(|e| MemtypeError::Unavailable {
        path: IOMEM,
        why: e.to_string(),
    })?;
    let end = base.saturating_add(len);
    // Every `System RAM` interval, and the deepest label covering `base`.
    let mut ram: Vec<(u64, u64)> = Vec::new();
    let mut label = String::new();
    let mut label_depth = usize::MAX;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let depth = line.len() - line.trim_start().len();
        let (range, name) =
            line.trim_start()
                .split_once(" : ")
                .ok_or_else(|| MemtypeError::Unparsable {
                    path: IOMEM,
                    line: line.trim().to_owned(),
                })?;
        let (lo, hi) = parse_hex_range(range).ok_or_else(|| MemtypeError::Unparsable {
            path: IOMEM,
            line: line.trim().to_owned(),
        })?;
        // `/proc/iomem` ends are INCLUSIVE. One off here silently mis-answers the last page
        // of every region, which is the page a window is most likely to end on.
        let hi = hi.saturating_add(1);
        if name == SYSTEM_RAM {
            ram.push((lo, hi));
        }
        if lo <= base && base < hi && (label.is_empty() || depth >= label_depth) {
            label = name.to_owned();
            label_depth = depth;
        }
    }
    Ok(PhysClass {
        system_ram: len > 0 && covered_entirely(base, end, &ram),
        label,
    })
}

/// Is `[base, end)` covered by the union of `ivals`? Sorts and sweeps, so adjacent
/// `System RAM` regions (which `/proc/iomem` does split) count as one.
fn covered_entirely(base: u64, end: u64, ivals: &[(u64, u64)]) -> bool {
    let mut v: Vec<(u64, u64)> = ivals.to_vec();
    v.sort_unstable();
    let mut at = base;
    for (lo, hi) in v {
        if lo > at {
            return false;
        }
        at = at.max(hi);
        if at >= end {
            return true;
        }
    }
    at >= end
}

fn parse_hex_range(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.trim().split_once('-')?;
    Some((
        u64::from_str_radix(a.trim(), 16).ok()?,
        u64::from_str_radix(b.trim(), 16).ok()?,
    ))
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Instrument 2 — the kernel's own record of what it installed
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **What memory type the kernel RECORDED for host-physical address `phys`.**
///
/// Reads `/sys/kernel/debug/x86/pat_memtype_list`, whose lines are
/// `PAT: [mem 0x<lo>-0x<hi>] <type>`, with `hi` exclusive. This is the kernel telling us
/// what it did, which is the only instrument here that reports rather than predicts.
///
/// `Ok(None)` means **no reservation covers this address**, and that is *not* the same as
/// write-back. Ordinary anonymous memory is untracked, so it answers `None`; so does an
/// address nothing ever mapped. The caller must supply the second fact
/// ([`classify_physical`]) to tell those apart, which is what [`effective_memtype`] does.
///
/// ⚠ **One way this file lies, and it is worth knowing.** The C artifact's `#111` fix
/// rewrote leaf page-table entries directly, behind the tracking layer's back, so that
/// file's answer and the hardware's behaviour disagreed. This instrument reports the
/// kernel's bookkeeping; a consumer that rewrites page tables itself has invalidated it.
/// Nothing in this port does that, and nothing may start without saying so here.
///
/// # Errors
/// [`MemtypeError::Unavailable`] if the file cannot be read (no `debugfs`, not x86, or
/// insufficient privilege — all three are ordinary and none is a defect);
/// [`MemtypeError::Unparsable`] on a `PAT:` line whose shape this parser does not know.
pub fn recorded_memtype(phys: u64) -> Result<Option<KernelMemtype>, MemtypeError> {
    let text = fs::read_to_string(PAT_LIST).map_err(|e| MemtypeError::Unavailable {
        path: PAT_LIST,
        why: e.to_string(),
    })?;
    let mut best: Option<(u64, KernelMemtype)> = None;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("PAT: [mem ") else {
            continue; // the header line, and anything else that is not an entry
        };
        let (range, ty) = rest
            .split_once(']')
            .ok_or_else(|| MemtypeError::Unparsable {
                path: PAT_LIST,
                line: line.trim().to_owned(),
            })?;
        let (lo, hi) = parse_hex_0x_range(range).ok_or_else(|| MemtypeError::Unparsable {
            path: PAT_LIST,
            line: line.trim().to_owned(),
        })?;
        let ty = KernelMemtype::parse(ty).ok_or_else(|| MemtypeError::Unparsable {
            path: PAT_LIST,
            line: line.trim().to_owned(),
        })?;
        if lo <= phys && phys < hi {
            // ★ Entries NEST: a driver's whole-BAR reservation and a caller's one-page
            // reservation both cover the address. The narrowest one is the one governing
            // this mapping, so width is the tie-break — not file order, which is arbitrary.
            let width = hi - lo;
            if best.is_none_or(|(w, _)| width < w) {
                best = Some((width, ty));
            }
        }
    }
    Ok(best.map(|(_, t)| t))
}

fn parse_hex_0x_range(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.trim().split_once('-')?;
    Some((
        u64::from_str_radix(a.trim().strip_prefix("0x")?, 16).ok()?,
        u64::from_str_radix(b.trim().strip_prefix("0x")?, 16).ok()?,
    ))
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The verdict
// ─────────────────────────────────────────────────────────────────────────────────────

/// The two structural facts and the verdict they support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveMemtype {
    /// What the caller asked for.
    pub requested: CachePolicy,
    /// `/proc/iomem`'s answer.
    pub phys: PhysClass,
    /// The kernel's own record, if it kept one.
    pub recorded: Option<KernelMemtype>,
    /// ★★★ The type the CPU actually uses, or `None` when no instrument here can say.
    /// `None` is an honest gap, never a pass.
    pub effective: Option<KernelMemtype>,
}

impl EffectiveMemtype {
    /// Whether the request survived.
    ///
    /// ★ An unknown [`Self::effective`] answers `false` — *"not known to have held"* is the
    /// honest reading, and a consumer that wants to proceed anyway must say so at its own
    /// call site rather than have this method say it for everyone.
    #[must_use]
    pub fn holds(&self) -> bool {
        self.effective
            .is_some_and(|e| e.as_cache_policy() == self.requested)
    }
}

/// ★★★ **Determine the effective memory type of a host-physical range — measured, not
/// assumed.**
///
/// The rule, in the order the kernel itself resolves it:
///
/// 1. If the kernel **recorded** a type for the address, that is the answer. It is the only
///    branch that reports rather than infers.
/// 2. Otherwise, if `/proc/iomem` calls the whole range `System RAM`, it is **write-back**:
///    untracked ordinary memory is exactly what the default type is for, and the downgrade
///    predicate does not apply to it.
/// 3. Otherwise the answer is **unknown** — a range that is neither tracked nor RAM is one
///    this instrument may not speak for. ⊘ It is *not* reported as write-back.
///
/// # Errors
/// [`MemtypeError::Unavailable`] / [`MemtypeError::Unparsable`] from either instrument.
/// ⚠ A missing PAT list is **not** fatal here: the `/proc/iomem` half still answers, so a
/// host without `debugfs` gets rule 2 or rule 3 rather than nothing.
pub fn effective_memtype(
    requested: CachePolicy,
    phys_base: u64,
    len: u64,
) -> Result<EffectiveMemtype, MemtypeError> {
    let phys = classify_physical(phys_base, len)?;
    let recorded = match recorded_memtype(phys_base) {
        Ok(r) => r,
        // The instrument is absent, not lying. Rule 2 and rule 3 still hold.
        Err(MemtypeError::Unavailable { .. }) => None,
        Err(e) => return Err(e),
    };
    let effective = match recorded {
        Some(t) => Some(t),
        None if phys.system_ram => Some(KernelMemtype::WriteBack),
        None => None,
    };
    Ok(EffectiveMemtype {
        requested,
        phys,
        recorded,
        effective,
    })
}

/// ★★★ **Refuse a mapping whose effective type is not the one that was asked for.**
///
/// This is [`crate::cache::require_attainable`]'s missing other half: that one refuses a
/// request the backing *cannot* satisfy, before the syscall; this one refuses an outcome the
/// kernel *did not* deliver, after it. A caller that runs only the first has checked its own
/// intent.
///
/// # Errors
/// - [`MemtypeError::Downgraded`] when the effective type is known and differs.
/// - The instrument errors, unchanged, when it could not read.
///
/// ⊘ An **unknown** effective type is returned as `Ok` with [`EffectiveMemtype::holds`]
/// false, never as a refusal — "I could not tell" and "it is wrong" are different facts and
/// merging them would make every host without `debugfs` look broken.
pub fn require_effective(
    requested: CachePolicy,
    phys_base: u64,
    len: u64,
) -> Result<EffectiveMemtype, MemtypeError> {
    let m = effective_memtype(requested, phys_base, len)?;
    if let Some(e) = m.effective
        && e.as_cache_policy() != requested
    {
        return Err(MemtypeError::Downgraded {
            requested,
            effective: e.as_cache_policy(),
            evidence: format!(
                "host-physical {phys_base:#x}+{len:#x} is {} and the kernel {}",
                if m.phys.system_ram {
                    "System RAM".to_owned()
                } else {
                    format!("{:?}, not System RAM", m.phys.label)
                },
                m.recorded
                    .map_or_else(|| "kept no record".to_owned(), |r| format!("recorded {r}"))
            ),
        });
    }
    Ok(m)
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Instrument 3 — the mapping itself, timed
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★ **The ratio below which a mapping is not uncached**, as a dimensionless number.
///
/// A cached load is a few nanoseconds; an uncached one is a bus transaction. The gap is
/// enormous and the threshold does not need to be delicate — what it needs is to be far
/// enough from `1.0` that scheduler noise cannot reach it, and far enough below the real
/// gap that a slow host cannot fall through it.
///
/// ★ **The number is `10` because that is what the margin actually is, not what it looks
/// like it should be.** Measured 2026-07-31 on one host with a real device aperture: a
/// direct compiled load gave a **602x** ratio, but the *same comparison through this
/// crate's checked accessors in an unoptimised build* gave **30.6x** — the wrapper's own
/// per-access cost sits in the numerator and the denominator and compresses the ratio by
/// twenty-fold. A floor chosen from the 602 would have been a floor the instrument itself
/// could not clear, which is the shape of a threshold that looks generous and is not.
pub const UNCACHED_RATIO_FLOOR: f64 = 10.0;

/// What a timing comparison says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandwidthVerdict {
    /// Indistinguishable from the cached reference.
    Cached,
    /// At least [`UNCACHED_RATIO_FLOOR`] times slower than the reference — uncached, in one
    /// of its forms. ★ It cannot say **which**: `UC-`, `UC` and `WC` are all uncached for
    /// loads, so a read-side witness may not name them apart and does not try.
    UncachedClass,
}

/// A timed read pass over a mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandwidthWitness {
    /// Nanoseconds per read.
    pub ns_per_read: f64,
    /// How many reads it took.
    pub reads: u64,
}

impl BandwidthWitness {
    /// Time `reads` invocations of `read`.
    ///
    /// ★ The closure takes the read **index**, so the caller strides its own mapping and
    /// this module never needs to hold a pointer or know the mapping's shape. That is what
    /// keeps this instrument outside the pointer surface entirely.
    ///
    /// # Panics
    /// If `reads == 0` — a rate over no samples is not a number.
    pub fn measure(reads: u64, mut read: impl FnMut(u64)) -> Self {
        assert!(reads > 0, "a bandwidth witness needs at least one read");
        let t0 = Instant::now();
        for i in 0..reads {
            read(i);
        }
        let ns = t0.elapsed().as_nanos();
        Self {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a duration in nanoseconds is far below f64's exact-integer range \
                          for any pass that finishes, and the result is a ratio"
            )]
            ns_per_read: ns as f64 / reads as f64,
            reads,
        }
    }

    /// Compare against a reference pass over memory known to be cached.
    ///
    /// ★★ **The reference is the whole instrument.** An absolute nanosecond figure means
    /// nothing across hosts, and a threshold in nanoseconds would be a constant that has to
    /// be re-tuned per box — which is a constant nobody re-tunes. A *ratio against a pass
    /// this same process just performed* carries the host's own speed inside it.
    #[must_use]
    pub fn against(self, cached_reference: Self) -> BandwidthVerdict {
        if self.ns_per_read >= cached_reference.ns_per_read * UNCACHED_RATIO_FLOOR {
            BandwidthVerdict::UncachedClass
        } else {
            BandwidthVerdict::Cached
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernels_two_uncached_spellings_stay_distinct_and_both_coarsen_to_uncached() {
        // The distinction that must survive parsing: the kernel writes two different words
        // and a downgrade produces the weak one.
        assert_eq!(
            KernelMemtype::parse("uncached-minus"),
            Some(KernelMemtype::UncachedMinus)
        );
        assert_eq!(
            KernelMemtype::parse("uncached"),
            Some(KernelMemtype::Uncached)
        );
        assert_ne!(KernelMemtype::UncachedMinus, KernelMemtype::Uncached);
        for t in [KernelMemtype::UncachedMinus, KernelMemtype::Uncached] {
            assert_eq!(t.as_cache_policy(), CachePolicy::Uncached);
        }
        // ...and the cached family coarsens the other way.
        for t in [
            KernelMemtype::WriteBack,
            KernelMemtype::WriteThrough,
            KernelMemtype::WriteProtected,
        ] {
            assert_eq!(t.as_cache_policy(), CachePolicy::WriteBack);
        }
        assert_eq!(
            KernelMemtype::WriteCombining.as_cache_policy(),
            CachePolicy::WriteCombining
        );
        // Every spelling round-trips through its own name.
        for t in [
            KernelMemtype::WriteBack,
            KernelMemtype::WriteThrough,
            KernelMemtype::WriteCombining,
            KernelMemtype::WriteProtected,
            KernelMemtype::UncachedMinus,
            KernelMemtype::Uncached,
        ] {
            assert_eq!(KernelMemtype::parse(t.as_str()), Some(t));
        }
        assert_eq!(KernelMemtype::parse("write-around"), None);
    }

    #[test]
    fn a_range_that_is_only_half_system_ram_is_not_system_ram() {
        // The `all`, not `any`, clause of the predicate — a window that runs off the end of
        // RAM is a window the downgrade applies to, and an `any` reading would pass it.
        let ram = [(0x1000, 0x2000), (0x2000, 0x3000)];
        assert!(covered_entirely(0x1000, 0x3000, &ram));
        assert!(covered_entirely(0x1800, 0x2800, &ram));
        assert!(!covered_entirely(0x1000, 0x3001, &ram));
        assert!(!covered_entirely(0x0fff, 0x2000, &ram));
        // Out of order, because /proc/iomem's order is not ours to rely on.
        let unsorted = [(0x2000, 0x3000), (0x1000, 0x2000)];
        assert!(covered_entirely(0x1000, 0x3000, &unsorted));
        // A gap is a gap however narrow.
        let holed = [(0x1000, 0x1fff), (0x2000, 0x3000)];
        assert!(!covered_entirely(0x1000, 0x3000, &holed));
    }

    #[test]
    fn an_unknown_effective_type_is_not_a_pass() {
        // The single most important behaviour in this module: "I could not tell" must never
        // read as "it held".
        let m = EffectiveMemtype {
            requested: CachePolicy::WriteBack,
            phys: PhysClass {
                system_ram: false,
                label: "nvidia".to_owned(),
            },
            recorded: None,
            effective: None,
        };
        assert!(!m.holds());
        // ...and a known-equal one does.
        let ok = EffectiveMemtype {
            effective: Some(KernelMemtype::WriteBack),
            ..m.clone()
        };
        assert!(ok.holds());
        // ...and a known-different one does not, including through the coarsening.
        let bad = EffectiveMemtype {
            effective: Some(KernelMemtype::UncachedMinus),
            ..m
        };
        assert!(!bad.holds());
    }

    #[test]
    fn the_bandwidth_witness_is_a_ratio_and_both_verdicts_are_reachable() {
        let reference = BandwidthWitness {
            ns_per_read: 2.0,
            reads: 1000,
        };
        let slow = BandwidthWitness {
            ns_per_read: 2.0 * UNCACHED_RATIO_FLOOR,
            reads: 1000,
        };
        let just_under = BandwidthWitness {
            ns_per_read: 2.0 * UNCACHED_RATIO_FLOOR - 0.001,
            reads: 1000,
        };
        assert_eq!(slow.against(reference), BandwidthVerdict::UncachedClass);
        assert_eq!(just_under.against(reference), BandwidthVerdict::Cached);
        assert_eq!(reference.against(reference), BandwidthVerdict::Cached);
        // The measurement itself counts what it was asked to. The instrument's
        // discriminating power is measured against a real device aperture in
        // `crates/kayfabe-linux-raw/tests/effective_memtype.rs`, not here.
        let w = BandwidthWitness::measure(64, |_| std::hint::black_box(()));
        assert_eq!(w.reads, 64);
        assert!(w.ns_per_read >= 0.0);
    }

    #[test]
    fn a_malformed_line_is_loud_rather_than_absent() {
        // Silence here would read as "untracked", which reads as "write-back". The parser
        // must not have a quiet failure mode.
        assert_eq!(parse_hex_0x_range("0x10-0x20"), Some((0x10, 0x20)));
        assert_eq!(parse_hex_0x_range("10-20"), None); // no 0x prefix
        assert_eq!(parse_hex_0x_range("0x10"), None); // no range
        assert_eq!(
            parse_hex_range("00001000-0009ffff"),
            Some((0x1000, 0x9ffff))
        );
        assert_eq!(parse_hex_range("zzz-1"), None);
    }
}
