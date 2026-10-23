# SPECviewperf results disclaimer

**STATUS: LIVE, 2026-10-23.** Applies to every SPECviewperf number in this directory.

> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

- SPECviewperf release: SPECviewperf 15.x (patch level not recorded)
- Configuration: Windows guest in a QEMU/KVM VM, fresh copy-on-write overlay of the owner image
  per trial; kayfabe kf3 virtual GPU, builds D/A/B/C as listed in builds.json; workload energy-04,
  Full HD, one iteration
- Ratios between two configurations are `(nc) research comparisons`, not SPEC-ranked
  results. Nothing here ranks against spec.org results or is an official score.
- Stamped (each score followed by `(nc)`, CSV files also start with a `#` header block
  holding the release, the configuration and the disclaimer): nothing: this directory holds no
  score table; its scores are the exported composites inside the run result.json files.
- Left unedited (raw logs and JSON evidence, kept as produced): every result.json and other JSON
  file, and the text files.
- Prose in README or comparison text may quote scores, ratios or percentages. Each such
  number is `(nc)` as well, whether or not the suffix is repeated after it.
- The SPEC release is not stated in the files; the headers say "SPECviewperf" without a version.
  Workload versions (for example 1.0.2) are not the release.
