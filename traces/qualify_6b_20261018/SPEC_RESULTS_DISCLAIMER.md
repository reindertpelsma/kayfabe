# SPECviewperf results disclaimer

**STATUS: LIVE, 2026-10-23.** Applies to every SPECviewperf number in this directory.

> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

- SPECviewperf release: SPECviewperf 15.x (patch level not recorded)
- Configuration: Windows guest (owner image) in a QEMU/KVM VM; kayfabe kf3 virtual GPU, candidate
  6b versus master 89fc8a222; VFIO physical-GPU reference arm copied from the vfio-ref results;
  Full HD
- Ratios between two configurations are `(nc) research comparisons`, not SPEC-ranked
  results. Nothing here ranks against spec.org results or is an official score.
- Stamped (each score followed by `(nc)`, CSV files also start with a `#` header block
  holding the release, the configuration and the disclaimer): comparison.md (score columns; the
  two ratio columns 6b/master and 6b/VFIO are headed as (nc) research comparisons; the counts in
  parentheses are sample counts, not scores), master-baseline-scores.csv,
  vfio-reference-scores.csv.
- Left unedited (raw logs and JSON evidence, kept as produced): comparison.json, every result.json
  and other JSON file, and the text files.
- Prose in README or comparison text may quote scores, ratios or percentages. Each such
  number is `(nc)` as well, whether or not the suffix is repeated after it.
- Both the 6b and the VFIO arms are virtualised.
- The SPEC release is not stated in the files; the headers say "SPECviewperf" without a version.
  Workload versions (for example 1.0.2) are not the release.
