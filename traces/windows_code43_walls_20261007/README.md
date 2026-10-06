# Windows Code43: one implementation gap per boot

**STATUS: RESEARCH, 2026-10-07.** Owner-directed continuation after
[exact request decoding](../windows_exact_decode_20261007/README.md). No Windows
success or master-promotion claim. Borrowed RTX4070; host595.91.07 remains attached
to its ordinary display driver. Only owned fresh-overlay Kayfabe VMs are used.

## First repair: Device-shared default VA resolution

The failing COPY2 channel has hVASpace=0 under client c1d00012. Its Device
explicitly shares client c1d00002's Device default. The old resolver searches only
the channel client's namespace and returns INVALID_STATE before channel creation.

The repair retains Device allocation facts in the graph, records a bounded
same-GPU Device-share relationship in ChannelPolicy, and resolves the canonical
VA's client and handle. Explicit channel, context-share and TSG declarations
retain their existing precedence. The existing mirror and channel birth checks
still decide whether actual GPU resources are ready; this does not bypass them.

Source: OGKM580.65.06 commit307159f2623d3bf45feb9177bd2da52ffbc5ddf9,
`src/nvidia/src/kernel/gpu/device_share.c:91` (share by client and GPU instance),
`src/nvidia/src/kernel/mem_mgr/vaspace.c:183` (implicit default permitted except
MULTIPLE_VASPACES). Mode constants are generated from OGKM580.159.04 headers by
`crates/kf-abi/gen/`; no captured per-die or host-privileged facts are added.

Missing/ambiguous targets, cross-GPU targets, unknown/private modes and cycles
refuse resolution. Traversal is capped at32 and declarations at the existing
MAX_LIVE_HANDLES budget. Target Device/client destruction revokes dependent
resolution; numeric handle reuse cannot revive it. Freeing the transient VA
handle retains the Device default, as the existing lifetime model requires.
This conservative support does not implement cross-Device VA reference retention
after the target Device is destroyed; existing live GPU channels retain their
ordinary resource lifetime and new births fail closed.

Regression tests exercise distinct Device handle values, canonical namespace,
local-default suppression for an explicit share, transient-VA free, target free,
handle reuse, missing targets, cross-GPU targets, explicit-VA-only mode and cycles.
The framebuffer MemoryList rejection and latency-buffer query are independent
remaining gaps; this iteration changes neither.

Local validation before the first hardware boot: all611 kf-rm tests pass,
including12 channel-link tests. Targeted kf-rm/kf-qemu Clippy reports zero new
debt (207 existing); formatting, diff and documentation claims checks pass.
