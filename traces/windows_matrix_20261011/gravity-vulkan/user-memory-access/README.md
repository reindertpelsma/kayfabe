# Validated live USERD memory access

**STATUS: LIVE, 2026-10-12. GPU-free adapter verified; exact Linux run pending.**

This shared object-seat adapter provides bounded worker access to the existing
validated RAM-backed MemoryList declarations. It holds current graph/resource
identity and RAM topology admission through each CPU copy. No RM verb, GPU
mapping, engine operation or completion is introduced. Linux user spaces remain
free of private host mappings. No runtime Windows USERD consumer exists yet.

MEASURED: full `cargo test -p kf-rm -p kf-qemu -p kf-chan -p kf-abi`
passed. Tests cover same-handle reallocation, parent/client destruction, graph
replacement while the old graph remains alive, logical bounds, alias-back
revival, metadata contention and real QEMU topology-writer contention.
The alias-back test failed before the fix (old lease incorrectly returned Ok).
The topology-contention test failed before the implementation (Unsupported
instead of Busy). Both passed afterward. The new authority defaults to
Unsupported, never a legacy blocking fallback. Independent source re-review
found no remaining blocker after both corrections.

Scope restriction: this initial worker adapter admits original client memory
allocations only, refusing aliases at acquisition and every access. General
MemoryList alias behavior remains unchanged. A future caller must validate the
channel/USERD ownership and migration ordering separately. Call Access outside
channel/relay transactions; never retry Busy while retaining their guards.

## Exact Linux regression — pre-run hypothesis

Hypothesis: sharing the live object graph and exposing the bounded nonblocking
worker seam preserves Linux object policy, USER isolation and existing fast arms.
Falsifier: any gate, USER privilege census, or original fast arm fails, crashes,
times out or is NOTRUN on the exact committed revision.

Run immutable `run_linux.sh FULL_REV` under the shared GPU lock. Rebuild the
revision-private QEMU and raw client; use a new disposable Linux guest. The
personal Windows image is not opened. Capture gate 9/9, gate10, fast30, binary
hash, host Xids (including expected negative-test faults), start/exit markers and
postconditions. Hardware result: NOT_RUN at this checkpoint.

Vast CLI checked at 05:15UTC: empty instance list. No instance needed or retired.
