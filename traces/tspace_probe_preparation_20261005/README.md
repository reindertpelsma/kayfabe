# USER/window probe preparation

**STATUS: RESEARCH, 2026-10-05.** Code **`e25e1bf9`** on
`codex/tspace-probes-2026-10-05`, based on integrated source `3d0959f9`.
No hardware, VM, rental or physical-mode GPU access was performed.

`scripts/p1p2/WINDOW_PROBE.md` gives the exact scope, source oracles and remaining
fixture requirements. This is executable preparation for T-WINDOW-USER, not a
passing T-WINDOW-USER, T-PHYS-CE, T-RING-TRANSLATED or S1-21 result.

Local commands used the shared Cargo lock, two build jobs, disabled incremental
compilation/debug info, and a distinct 74 MiB target:

```sh
flock /tmp/kayfabe-cargo.lock env CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=/tmp/kayfabe-tspace-probes-target \
  cargo test -p kf-harness --lib --bin kf-window-probe
flock /tmp/kayfabe-cargo.lock env CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=/tmp/kayfabe-tspace-probes-target \
  cargo build -p kf-harness --bin kf-window-probe
/tmp/kayfabe-tspace-probes-target/debug/kf-window-probe --help
cargo fmt --all --check
git diff --check
```

- `tests.log`: four tests pass. Their negative cases cover timeout-only claims,
  incorrect exception/status/engine, leaked destination data, completed work
  after an alleged denial, bad baselines, dead neighbors, unknown/duplicate or
  stale manifest inputs, extent overflow/alignment/edge bounds, and every
  truncation of the error-notifier decoder.
- `build.log`: executable builds. `--help` was the only executable invocation;
  it exits before identity/GPU discovery.
- `unsafe-gate.log`: the actual CI containment/ratchet script passes. No unsafe
  code or new allowance was introduced. Formatting and whitespace checks pass.

The tests and build ran on the working tree committed as `e25e1bf9`; later
changes in this evidence commit are documentation/logs only. Logs retain
compiler warnings. Existing broad integration tests are under
`traces/windows_p1p2_integration_20261005/`; their counts are not repeated or
claimed to validate this new hardware probe.

The first physical-machine step, owned by the parent task, is the unprivileged
native self-test on an exclusive GPU, retaining exact revision, UID/caps,
driver/GPU identity, USER birth replies, notifier records, neighbor completions
and host Xids. An outer timeout is a failed/inconclusive run. No successful
hardware observation has yet been made by this branch.
