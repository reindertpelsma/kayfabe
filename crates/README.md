# crates/

**STATUS: LIVE, 2026-11-23.** Which crates are the product, which are oracles and which are frozen. The one-line description of each `kf-*` crate is the table in [`ARCHITECTURE.md`](../ARCHITECTURE.md); the rules (where `unsafe` may live, what may not depend on what) are in [`AGENTS.md`](../AGENTS.md).

| Group | Crates | Rule |
|---|---|---|
| **v3 product** | `kf-abi`, `kf-arch`, `kf-broker`, `kf-chan`, `kf-chip`, `kf-core`, `kf-cuda`, `kf-disp`, `kf-gsp`, `kf-host`, `kf-linux-raw`, `kf-mem`, `kf-oprom`, `kf-qemu`, `kf-rm`, `kf-trap`, `kf-util`, `kf-gop-image` | `cargo test -p kf-<crate>` is GPU-free. `unsafe` only in `kf-linux-raw`, `kf-qemu` and `kf-cuda`, in files named `*_unsafe.rs`. |
| **Oracle and tools, not linked into the product** | `kf-crec`, `kf-trace` (C-trace replays), `kf-census` (offline capture decoder), `kf-harness` (the GPU gates `kf-gate1..9`), `kf-rawclient` (raw RM client) | Never a dependency of a product crate. |
| **Frozen pre-v3 tree** | every `kayfabe-*` directory here | Reference and the grader only. `kayfabe-rm-ladder` and its dependency closure must keep building; add no features; no `kf-*` crate may depend on one. |

The pre-v3 crates the grader does not reach are in [`archive/crates/`](../archive/README.md) and do not build.
