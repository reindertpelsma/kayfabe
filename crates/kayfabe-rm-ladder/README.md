# kayfabe-rm-ladder

**STATUS: FROZEN, 2026-11-23.** Part of the pre-v3 tree, kept as reference. Do not add features here, and never make a `kf-*` crate depend on it (`AGENTS.md`, *Layout*). The v3 product is the `kf-*` crates; see [`crates/README.md`](../README.md) and [`ARCHITECTURE.md`](../../ARCHITECTURE.md).

This crate is the 30-arm raw test client used by `scripts/fastguest/` and the bare-metal suite; it and its dependency closure must keep building.
