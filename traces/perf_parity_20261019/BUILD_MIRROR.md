# Mirror-based P1 immutable host build

STATUS: MEASURED, 2026-10-19 02:48 CEST. CPU-only; GPU lease not acquired.

MEASURED: source `41a8beb07573847a932b76d48bc0dcd4383ea247` was pushed and verified on origin before the trusted-host build. Canonical `scripts/bench/build_kf3.sh` ran under nice 19, cargo/ninja jobs limited to two, private target/cache/temp directories, and Cargo `--locked`. No existing binary directory was replaced.

MEASURED: binary `/data/codex-P1/kf3-bins/41a8beb0/qemu-system-x86_64`; SHA-256 `653d8d8b7c59f62b8998078356288111dfbd88776c67f1fff05c72a346f5ab08`. Build log `/data/codex-P1/build-41a8beb0.log`. Cargo features: `kf-qemu/diag-nsi,kf-qemu/diag-get-writeback`. Runtime diagnostic flags are unset by default. Gate binaries and the raw client were rebuilt from the same source. This supersedes the earlier production-based P1 binary for hardware A/B.

MEASURED: the meter broker was built and checked separately from exact mirror source `c2685412e`: `/data/codex-P1/broker-c2685412/broker/nvkvm-display-broker`, SHA-256 `0ecc0db3d8b13aae16696f4f07e54c33a877d95750b70fe64eb9b173d28b0165`. Its compiler-derived content-counter offset is 360. All Windows measurement arms use this same broker and `--stats-line`. CPU broker checks passed; this does not claim guest hardware validation.

CODE READ: production builds omit both diagnostic features. The measurement binary requires explicit runtime opt-in to trace or suppress ordinary GET writes. Hardware gates and performance measurements are still pending; no morning promotion is established.
