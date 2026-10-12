# Validated live USERD memory access

**STATUS: LIVE, 2026-10-12. GPU-free adapter and exact Linux regression verified.**

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
postconditions. MEASURED on exact `7f3f9fe16c6d97513c0c9406ddcdf011efca5937`, trusted
RTX4070 / host595.91.07, 05:18:01–05:22:17UTC: gates9/9 and gate10 PASS,
USER births12/12, refused0; rebuilt QEMU/client/new disposable guest; fast30/30
PASS, crashes0, NOTRUN0, exit0. Five host Xid31 match negative-test fault
addresses (0x60/0x62/0x64 unmapped reads, 0xa_11000000 write); no Xid32.
This is not a zero-Xid claim. Full text logs and binary SHA256 in linux-result.
Postconditions: no QEMU, DMA-FQ, lock free, stop marker absent,117392404480 bytes
free, personal original image0444 and28240642048 bytes unchanged. No Windows or
subsequent source claim is supplied by this Linux run.

Vast CLI checked at 05:15UTC: empty instance list. No instance needed or retired.
