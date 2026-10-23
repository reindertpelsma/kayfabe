# SPECviewperf results disclaimer

**STATUS: LIVE, 2026-10-23.** Applies to every SPECviewperf number in this directory.

> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

- SPECviewperf release: SPECviewperf 15.x (patch level not recorded)
- Configuration: Windows guest (owner image) in a QEMU/KVM VM, 12 GiB RAM, 8 vCPUs, Full HD; VFIO
  arm = physical GeForce RTX 4070 passed through; kayfabe arm = kf3 virtual GPU (details in
  README.md of this directory)
- Ratios between two configurations are `(nc) research comparisons`, not SPEC-ranked
  results. Nothing here ranks against spec.org results or is an official score.
- Stamped (each score followed by `(nc)`, CSV files also start with a `#` header block
  holding the release, the configuration and the disclaimer): vfio-composites.csv,
  vfio-first-workload.csv, vfio-subtest-fps.csv, kayfabe-partial-fps.csv, paired-subtests.csv
  (column vfio_over_kayfabe is a ratio), and the README.md table.
- Left unedited (raw logs and JSON evidence, kept as produced): vfio-run1-vendor-results.csv,
  vfio-run2-vendor-results.csv (vendor CSV copies), composite-summary.json,
  kayfabe-energy-score.json, native-*.txt and every other JSON or text file.
- Prose in README or comparison text may quote scores, ratios or percentages. Each such
  number is `(nc)` as well, whether or not the suffix is repeated after it.
- Ratio column `vfio_over_kayfabe` is headed as a (nc) research comparison. Both arms are
  virtualised; some kayfabe rows come from workloads that failed or did not complete.
- The SPEC release is not stated in the files beyond "SPECviewperf15"; workload versions (for
  example 1.0.1) are not the release.
