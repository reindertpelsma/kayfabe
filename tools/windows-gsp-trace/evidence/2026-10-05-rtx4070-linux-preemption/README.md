# Native Linux graphics-preemption mode experiment

**STATUS: RESEARCH, 2026-10-05.** RTX 4070 (AD104), native Linux 7.0.0-31-generic,
NVIDIA open 595.91.07. This is native Linux behavior, not a Kayfabe guest result.

Source: `linux-preemption-mode.c` at research commit `0af50483`, plus the repository's
`vk_probe.c` and `egl_offscreen.c`; exact source hashes are in `source-sha256.txt`.
The shim issues a separate `SET_CTXSW_PREEMPTION_MODE` on the application's owned channel,
with flags=2 (graphics selected) and mode 0, 1, or 2. It preserves the application's
original ioctl result. Requests ran as UID/GID 65534 with supplementary groups and all
capabilities dropped and no-new-privileges set. No privileged RM request is forwarded.

| Workload | Additional graphics request | RM result | Workload result |
|---|---|---|---|
| Vulkan baseline | none | — | 6/6 checks |
| Vulkan mode 0 (WFI) | 4 calls | 4 NV_OK | 6/6 checks |
| Vulkan mode 1 (non-pooled GfxP) | 4 calls | 3 NV_OK, 1 `NV_ERR_NO_MEMORY` (0x51) | 6/6 checks |
| Vulkan mode 2 (pooled GfxP) | 4 calls | 4 `NV_ERR_INVALID_ARGUMENT` (0x1f) | 6/6 checks |
| Headless OpenGL baseline | none | — | CHECK ok, digest `6ab6ddbcd55aa8bb` |
| Headless OpenGL mode 1 | 1 call | NV_OK | Same digest, CHECK ok |
| Headless OpenGL mode 2 | 1 call | `NV_ERR_INVALID_ARGUMENT` | Same digest, CHECK ok |

The OpenGL renderer identifies the physical RTX 4070; this is not a software-renderer
success. Its command was `egl_offscreen 2 64`, with NVIDIA's EGL vendor selected.
The `.err` files record each additional request and the renderer; `.out` records workload
checks; `.rc` is the process exit status. Decimal status 81 is **out of memory**, not
“not supported”; its cause was not diagnosed.

This establishes that an unprivileged application can request non-pooled graphics GfxP
successfully on a real graphics channel and still render the checked frame. It does **not**
measure preemption latency, prove that preemption occurred under load, or show a successful
Linux pooled-preemption path. A successful workload after mode 2 was refused is not a
successful pooled-mode test. It does not establish Windows compatibility or other dies.

An attempted bpftrace probe of `rpcRmApiControl_GSP` could not attach: the kernel exposed
the symbol but not a matching probe. `gsp-controls.err` records that failure. There is no
GSP-boundary capture here and no claim that this experiment proved a pool query absent.

Reproduction: compile the shim as documented in its source, then launch each workload
with `KF_GFX_MODE=0|1|2 LD_PRELOAD=/absolute/path/mode.so` under an unprivileged identity.
Baseline omits the shim. Mode changes are scoped to the application's own channels.
