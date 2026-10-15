# V3 — USER-ring store views: keep them armed (the D3D11 view churn)

**STATUS: LIVE on branch `claude/perf-fix-20261015`, 2026-10-15 (built and measured: see `V3_WINDOWS_PERF_20261015.md`).** Labels: **[measured]** = a number from a run named in the perf doc; **[read]** = kayfabe code; **[inferred]** = reasoning.

## 1. The defect

`userstore::Cache::prepare` (the worker-owned cache of CPU views over the framebuffer store that the USER-ring relay reads GPFIFO entries and pushbuffer segments through) released every ready view that the NEXT snapshot did not require, and armed the missing ones. A Windows D3D11 channel alternates between a few 64 KiB store windows on nearly every submission, so each snapshot cost one arm + one release: `openat(/dev/nvidia0)`, `NV_ESC_RM_MAP_MEMORY`, `mmap`, `munmap`, `NV_ESC_RM_UNMAP_MEMORY`, `close`, with a GPU TLB invalidate on the host. **[measured, Heaven D3D11, production flags]** ~16 500 such cycles per second on the two workers, 106 s of SYSTEM time per worker per 293 s, GPU 28 % busy, 20.9 fps (OpenGL on the same boot: 229 fps, ~523 cycles/s).

## 2. The change

Views are kept. A view is released only (a) when the channel is at its limit and a new window needs a slot (least recently used first, never one the current read needs) or (b) when RM refuses a new mapping. Each view stays owned by ONE cache (one channel; the cache is `&mut` inside the channel's pump, one worker at a time), so no reader can see a view that another thread unmapped, and there is no lock, no blocking call and no new shared structure. The only shared state is process-wide counters for the summary line.

RM refusing a mapping (out of BAR1, any error): the channel releases its own least recently used view and retries once; if that fails it falls back to today's behaviour for that read (release every view the read does not need, arm); if even that fails the read is refused by name (`Refusal::Store`, as before). A mapping that exists but could not be mapped is retained, never discarded. Nothing is derived from the host's BAR1 size or free space.

## 3. The property (QOM, ABI 30)

| property | type | default | meaning |
|---|---|---|---|
| `user-view-keep` | u32 | 0 = 16 | CPU views of the store (64 KiB each, host BAR1) each USER-ring channel keeps armed. 16 is the previous per-channel quota (1 MiB). Operator limit, maximum 256. Raise it if the shutdown line `USER_STORE_VIEWS channel ... evicted_at_quota=N` names a channel. |

Set like the others: `-device kf3-gpu,...,user-view-keep=32`. **[measured]** the Heaven D3D11 working set is 1-5 windows per channel, none shared between channels (offline LRU replay: 4 views per channel = 99.94 % hits, 8 = 100 %), so the default is ample.

## 4. Counters, by name

At shutdown (next to `USER_RING_LOOP SUMMARY`): `kf3: USER_STORE_VIEWS keep=16 armed=<n> evicted_at_quota=<n> rm_refused_retry_ok=<n> rm_refused_fallback_ok=<n> rm_refused_read_refused=<n>`. When a channel is released with evictions: `kf3: USER_STORE_VIEWS channel (hClient, hObject) armed=.. evicted_at_quota=.. keep=..`. Each RM-refusal event logs `USER_STORE_VIEW <kind> #n` (the first 8, then every power of two).

## 5. Tests (GPU-free, `cargo test -p kf-qemu --lib userstore`)

Unit (stub backend with arm/release/refusal injection): alternating windows are armed once, not per snapshot (the churn case); N channels x M slices arm each slice once; LRU order beyond the limit; a snapshot larger than the limit is satisfied in full; RM refusal -> own LRU eviction + one retry; two refusals -> arm/release fallback; three -> refused by name with a consistent cache; a failed release keeps ownership and the view is never read again; a failed mmap retains the RM mapping.

Stress (`eight_threads_arm_evict_and_read_real_mappings_with_injected_failures`): 8 threads, each with its own cache over a stub that creates a REAL memfd and mmaps it per view (a read through a released view would SIGSEGV), every byte of every read checked against an offset-derived pattern, 3 % arm refusals, partial mappings and release refusals injected, one thread hammering distinct windows; at the end arms - releases = 0, live mappings 0, `/proc/self/maps` and fd counts back to baseline. Seeds are printed (`KF3_STRESS_SEED`, `KF3_STRESS_SECS`, `KF3_STRESS_THREADS` replay or lengthen a run). The first version of the stress run found a real defect in this change (a retry path discarded the RM mapping that a failed mmap had left behind: rm_maps 5311 vs rm_unmaps 5309) and the fix is `try_arm`.

## 6. What this does not do

No shared pool and no lock-free table: the measured working sets show no cross-channel sharing, so there is nothing to gain. The view COPIES remain uncached BAR1 loads (D3D11: 867 per frame, 96-416 bytes each, 35 % above 256 bytes, ~16 ns per byte); see the perf doc for the next bottleneck.
