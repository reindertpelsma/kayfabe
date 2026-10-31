//! The run census and the qualification: **a workload that fails it decides nothing**.
//!
//! The refresh stress is only a stress of the host stitch if its pages are physically scattered, one
//! `MAP_FIXED` piece per page. So before anything is pinned the arm reads, for every batch in mapping
//! order, the frame number of each page and counts maximal runs of consecutive frames:
//! `runs = 1 + #{i : pfn[i+1] != pfn[i] + 1}`. `run_ratio = runs / pages` is 1.0 when no two neighbours
//! in mapping order are neighbours in memory and tends to 0 when the batch is one contiguous run.
//! The batch must also have all pages present, no transparent huge page backing, and frame numbers that
//! are real (an unprivileged `pagemap` reads zero, which is blindness and not contiguity).

use crate::outcome::NotRunReason;
use std::collections::BTreeSet;

/// 4 KiB frames per 2 MiB region.
const FRAMES_PER_2M: u64 = 512;
/// Bytes per frame the kernel's pagemap counts in (guarded by `SmallPageMatchesLeaf`).
const FRAME_BYTES: u64 = 4096;

/// What `pagemap` said about one page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageFrame {
    /// The page is in RAM.
    pub present: bool,
    /// Its frame number (0 when hidden).
    pub pfn: u64,
}

/// The census of one refresh's workload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    /// Pages examined.
    pub pages: u64,
    /// Sum of per-batch run counts.
    pub runs: u64,
    /// Longest physically contiguous run, pages.
    pub max_run_len: u64,
    /// Distinct 2 MiB regions touched.
    pub distinct_2m: u64,
    /// Pages not present in RAM.
    pub missing: u64,
    /// Present pages whose frame reads 0.
    pub zero_pfn: u64,
    /// Lowest per-batch `runs * 1000 / pages`.
    pub min_batch_permille: u32,
    /// Frame span from lowest to highest present frame, MiB.
    pub span_mib: u64,
}

impl Census {
    /// `runs / pages` over the whole workload.
    #[must_use]
    pub fn run_ratio(&self) -> f64 {
        if self.pages == 0 {
            0.0
        } else {
            self.runs as f64 / self.pages as f64
        }
    }
}

/// Accumulates batches in mapping order.
#[derive(Debug, Default)]
pub struct CensusBuilder {
    pages: u64,
    runs: u64,
    max_run: u64,
    regions: BTreeSet<u64>,
    missing: u64,
    zero_pfn: u64,
    min_permille: Option<u32>,
    lo: Option<u64>,
    hi: u64,
}

impl CensusBuilder {
    /// A fresh builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the next batch's frames, in mapping order. An absent page breaks a run.
    pub fn add_batch(&mut self, frames: &[PageFrame]) {
        if frames.is_empty() {
            return;
        }
        let mut runs = 0u64;
        let mut cur = 0u64;
        let mut prev: Option<u64> = None;
        for f in frames {
            self.pages += 1;
            if !f.present {
                self.missing += 1;
                prev = None;
                cur = 0;
                continue;
            }
            if f.pfn == 0 {
                self.zero_pfn += 1;
            } else {
                self.regions.insert(f.pfn / FRAMES_PER_2M);
                self.lo = Some(self.lo.map_or(f.pfn, |l| l.min(f.pfn)));
                self.hi = self.hi.max(f.pfn);
            }
            match prev {
                Some(p) if f.pfn == p.wrapping_add(1) => cur += 1,
                _ => {
                    runs += 1;
                    cur = 1;
                }
            }
            self.max_run = self.max_run.max(cur);
            prev = Some(f.pfn);
        }
        self.runs += runs;
        let permille = u32::try_from(runs * 1000 / frames.len() as u64).unwrap_or(u32::MAX);
        self.min_permille = Some(self.min_permille.map_or(permille, |m| m.min(permille)));
    }

    /// Finish.
    #[must_use]
    pub fn finish(self) -> Census {
        Census {
            pages: self.pages,
            runs: self.runs,
            max_run_len: self.max_run,
            distinct_2m: self.regions.len() as u64,
            missing: self.missing,
            zero_pfn: self.zero_pfn,
            min_batch_permille: self.min_permille.unwrap_or(0),
            span_mib: self
                .lo
                .map_or(0, |lo| ((self.hi - lo + 1) * FRAME_BYTES) >> 20),
        }
    }
}

/// The verdict on a census.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Qualification {
    /// The workload is fragmented as requested.
    Qualified,
    /// It is not (or could not be read): the run decides nothing.
    NotRun(NotRunReason, String),
}

/// Judge `c` against the floor `min_permille`, given the THP reading of the region.
#[must_use]
pub fn qualify(c: &Census, min_permille: u32, thp_bytes: Option<u64>) -> Qualification {
    let nq = |detail: String| Qualification::NotRun(NotRunReason::NotQualified, detail);
    if c.pages == 0 || c.missing == c.pages {
        return nq(format!("pages-not-present n={}", c.missing));
    }
    if c.zero_pfn == c.pages - c.missing {
        return Qualification::NotRun(
            NotRunReason::PagemapPfnHidden,
            "present pages read frame 0".into(),
        );
    }
    if c.missing > 0 {
        return nq(format!("pages-not-present n={}", c.missing));
    }
    if c.zero_pfn > 0 {
        return nq(format!("frames-partly-hidden n={}", c.zero_pfn));
    }
    match thp_bytes {
        None => {
            return Qualification::NotRun(
                NotRunReason::ThpUnmeasurable,
                "smaps named no VMA for the region".into(),
            );
        }
        Some(b) if b > 0 => return nq(format!("thp-bytes={b}")),
        Some(_) => {}
    }
    if c.min_batch_permille < min_permille {
        return nq(format!(
            "run-ratio batch_min={} required={}",
            c.min_batch_permille, min_permille
        ));
    }
    Qualification::Qualified
}

/// The `STRESS_QUALIFY` line.
#[must_use]
pub fn line(
    c: &Census,
    thp_bytes: Option<u64>,
    mem_avail_mib: u64,
    seed: u64,
    required_permille: u32,
    q: &Qualification,
) -> String {
    format!(
        "STRESS_QUALIFY pages={} runs={} run_ratio={:.4} thp={} distinct_2m={} span_mib={} mem_avail_mib={mem_avail_mib} seed={seed:#x} max_run_len={} min_batch_ratio={:.3} required={:.3} result={}",
        c.pages,
        c.runs,
        c.run_ratio(),
        match thp_bytes {
            Some(0) => "off".to_owned(),
            Some(b) => format!("{b}"),
            None => "unmeasured".to_owned(),
        },
        c.distinct_2m,
        c.span_mib,
        c.max_run_len,
        f64::from(c.min_batch_permille) / 1000.0,
        f64::from(required_permille) / 1000.0,
        match q {
            Qualification::Qualified => "qualified".to_owned(),
            Qualification::NotRun(r, _) => r.word().to_owned(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(pfns: &[u64]) -> Vec<PageFrame> {
        pfns.iter()
            .map(|&pfn| PageFrame { present: true, pfn })
            .collect()
    }

    fn census(batches: &[Vec<PageFrame>]) -> Census {
        let mut b = CensusBuilder::new();
        for x in batches {
            b.add_batch(x);
        }
        b.finish()
    }

    #[test]
    fn the_run_formula_is_one_plus_the_breaks() {
        // 100,101,102 | 200 | 300,301 -> 3 runs in 6 pages.
        let c = census(&[frames(&[100, 101, 102, 200, 300, 301])]);
        assert_eq!((c.pages, c.runs, c.max_run_len), (6, 3, 3));
        assert_eq!(c.min_batch_permille, 500);
        // Fully contiguous: one run. Fully scattered: one run per page.
        let c = census(&[frames(&(1000..1064).collect::<Vec<_>>())]);
        assert_eq!((c.runs, c.max_run_len, c.min_batch_permille), (1, 64, 15));
        let scattered: Vec<u64> = (0..64).map(|i| 5000 + i * 7).collect();
        let c = census(&[frames(&scattered)]);
        assert_eq!((c.runs, c.max_run_len, c.min_batch_permille), (64, 1, 1000));
        // Descending neighbours are not a run: mapping order matters, not adjacency alone.
        let desc: Vec<u64> = (0..16).rev().map(|i| 9000 + i).collect();
        assert_eq!(census(&[frames(&desc)]).runs, 16);
    }

    #[test]
    fn runs_do_not_span_batches_and_the_floor_is_per_batch() {
        // Two batches that would join into one run if the boundary were ignored.
        let c = census(&[frames(&[10, 11]), frames(&[12, 13])]);
        assert_eq!(c.runs, 2);
        // One fragmented batch cannot hide a contiguous one: the minimum is what qualifies.
        let good: Vec<u64> = (0..100).map(|i| 3 * i + 7).collect();
        let bad: Vec<u64> = (0..100).map(|i| 50_000 + i).collect();
        let c = census(&[frames(&good), frames(&bad)]);
        assert_eq!(c.min_batch_permille, 10);
        assert_eq!(c.run_ratio(), 101.0 / 200.0);
        assert!(matches!(
            qualify(&c, 900, Some(0)),
            Qualification::NotRun(NotRunReason::NotQualified, _)
        ));
    }

    #[test]
    fn region_span_and_max_run_are_reported() {
        let c = census(&[frames(&[0x1000, 0x1001, 0x1002, 0x9000, 0x20_0000])]);
        assert_eq!(c.max_run_len, 3);
        assert_eq!(c.distinct_2m, 3);
        assert_eq!(c.span_mib, ((0x20_0000 - 0x1000 + 1) * 4096) >> 20);
    }

    #[test]
    fn qualification_requires_every_condition_and_names_the_failing_one() {
        let scattered: Vec<u64> = (0..1000).map(|i| 10_000 + i * 3).collect();
        let good = census(&[frames(&scattered)]);
        assert_eq!(qualify(&good, 900, Some(0)), Qualification::Qualified);
        let reason = |q: Qualification| match q {
            Qualification::NotRun(r, d) => (r, d),
            Qualification::Qualified => panic!("qualified"),
        };
        // THP present, THP unmeasured.
        assert_eq!(
            reason(qualify(&good, 900, Some(2 << 20))).0,
            NotRunReason::NotQualified
        );
        assert_eq!(
            reason(qualify(&good, 900, None)).0,
            NotRunReason::ThpUnmeasurable
        );
        // Not fragmented enough.
        let contig = census(&[frames(&(0..1000).map(|i| 777 + i).collect::<Vec<_>>())]);
        let (r, d) = reason(qualify(&contig, 900, Some(0)));
        assert_eq!(r, NotRunReason::NotQualified);
        assert!(d.starts_with("run-ratio"), "{d}");
        // The floor is a parameter: lowering it to the achieved ratio qualifies.
        assert_eq!(qualify(&contig, 1, Some(0)), Qualification::Qualified);
        // A missing page.
        let mut f = frames(&scattered);
        f[17].present = false;
        let (r, d) = reason(qualify(&census(&[f]), 900, Some(0)));
        assert_eq!(
            (r, d.as_str()),
            (NotRunReason::NotQualified, "pages-not-present n=1")
        );
        // Nothing present at all.
        let none = census(&[vec![PageFrame::default(); 8]]);
        assert_eq!(
            reason(qualify(&none, 900, Some(0))).0,
            NotRunReason::NotQualified
        );
        // An empty census decides nothing either.
        assert_eq!(
            reason(qualify(&census(&[]), 900, Some(0))).0,
            NotRunReason::NotQualified
        );
    }

    #[test]
    fn hidden_frames_are_blindness_never_contiguity_or_fragmentation() {
        let hidden = census(&[vec![
            PageFrame {
                present: true,
                pfn: 0
            };
            64
        ]]);
        let q = qualify(&hidden, 900, Some(0));
        assert!(
            matches!(q, Qualification::NotRun(NotRunReason::PagemapPfnHidden, _)),
            "{q:?}"
        );
        // A few zero frames among real ones are reported as a partial hide, not ignored.
        let mut f = frames(&(0..64).map(|i| 100 + 3 * i).collect::<Vec<_>>());
        f[3].pfn = 0;
        let q = qualify(&census(&[f]), 900, Some(0));
        assert!(
            matches!(q, Qualification::NotRun(NotRunReason::NotQualified, _)),
            "{q:?}"
        );
    }

    #[test]
    fn the_line_has_the_documented_fields_in_order() {
        let scattered: Vec<u64> = (0..16).map(|i| 4096 + i * 5).collect();
        let c = census(&[frames(&scattered)]);
        let l = line(&c, Some(0), 2000, 0xabc, 900, &Qualification::Qualified);
        assert!(
            l.starts_with("STRESS_QUALIFY pages=16 runs=16 run_ratio=1.0000 thp=off distinct_2m="),
            "{l}"
        );
        for k in [
            "span_mib=",
            "mem_avail_mib=2000 seed=0xabc",
            "max_run_len=1",
            "required=0.900",
            "result=qualified",
        ] {
            assert!(l.contains(k), "{k} in {l}");
        }
        let l = line(
            &c,
            None,
            1,
            1,
            900,
            &Qualification::NotRun(NotRunReason::ThpUnmeasurable, String::new()),
        );
        assert!(l.contains("thp=unmeasured") && l.ends_with("result=thp-unmeasurable"));
    }
}
