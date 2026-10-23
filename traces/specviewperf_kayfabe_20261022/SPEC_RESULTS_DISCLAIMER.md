# SPECviewperf results disclaimer

**STATUS: LIVE, 2026-10-23.** Applies to every SPECviewperf number in this directory.

> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

- SPECviewperf release: SPECviewperf 15.x (patch level not recorded)
- Configuration: Windows guest (owner image) in a QEMU/KVM VM; arms: kayfabe kf3 virtual GPU
  (baseline binary 89fc8a222, Phase 1 binary 468a35c9d) and a VFIO physical-GPU reference (GeForce
  RTX 4070, see the vfio-ref branch); host GPU of the kayfabe arms is not restated here
- Ratios between two configurations are `(nc) research comparisons`, not SPEC-ranked
  results. Nothing here ranks against spec.org results or is an official score.
- Stamped (each score followed by `(nc)`, CSV files also start with a `#` header block
  holding the release, the configuration and the disclaimer): baseline-scores.csv, fix-scores.csv,
  vfio-composites.csv, baseline-subtests.csv, fix-subtests.csv, comparison.csv (columns *_over_*
  are ratios), and the tables in comparison.md and README.md.
- Left unedited (raw logs and JSON evidence, kept as produced): every JSON file, build.txt,
  binary-hashes.txt, baseline-budget-excerpts.txt and the other text files.
- Prose in README or comparison text may quote scores, ratios or percentages. Each such
  number is `(nc)` as well, whether or not the suffix is repeated after it.
- Ratio columns (`*_over_*`, "Phase 1 / VFIO mean") are headed as (nc) research comparisons. A
  blank cell or a dash means no qualified composite exists.
- The SPEC release is not stated in the files beyond "SPECviewperf 15"; workload versions (for
  example 1.0.1) are not the release.
