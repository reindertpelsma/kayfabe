# ENS3 diagnostic validation

**STATUS: GPU-free validation, 2026-10-26.** Source branch `codex/ens3-20261026`.
The exact sealed head and controller build authorization are recorded in the
private lane handoff. No Windows/GPU result is claimed here.

MEASURED: `expected-red.txt` records the failing-first snapshot/timing tests at
`ee5f7eda6` and CPU-clock tests at `4e55d261b`; the combined base is `3718a77e9`.
The snapshot and metric API scaffolding is intentionally inert in the first RED.

MEASURED (mutation checks, 2026-10-26): `mutants.json` records17 mutants, each killed by at least two named
tests. Functional tests exercise parser, one-shot capture, history/selection,
ring progress, bounds, disjoint timing and Linux CPU-clock behavior. Composition
guards also prove that real apply, sparse, publication and RM call sites remain
wired to the recorder; those guards are explicitly identified as `hooks`.

MEASURED (dev probe, 2026-10-26): `admission-probe.txt` records one deterministic GPU-free reservation
model run with8192 blocks and an early budget refusal, diagnostics enabled. It
counts4194304 page visits and zero fake-RM calls. Timings are dev-box n=1, not a
Windows timing claim or a benchmark score. Sparse behavior is unchanged.

The complete final test summary will be in `validation.txt`. The standard QEMU
build/install step and hardware schedule remain controller-owned and deferred.
