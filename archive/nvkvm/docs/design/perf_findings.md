# Decode-Latency Perf Findings (host vs guest)

Status: IN PROGRESS (2026-05-31). Measurements + probable fixes for the
single-stream LLM-decode latency gap. Companion to `fastpath_command_buffer`
discussion and memory `decode_ioctl_profile` / `perf_host_vs_guest`.

## Measurements

1. **Compute/data path = ~0% overhead.** GEMM throughput host 450.5 vs guest
   452.3 GFLOP/s (within noise). The passthrough-DMA-mmap design makes sustained
   compute bare-metal. (`tests/integration/gpu_bench.c`)

2. **Single-stream decode is heavily penalized.** Qwen-0.5B: host 368 t/s vs
   guest ~24 t/s decode (~15×); host 800 vs guest 274 t/s prefill (~2.9×). The
   guest decode rate is ~flat across model size (0.5B ~24, 7B ~20) and across
   CUDA-graphs on/off → a fixed per-token *latency floor*, not a compute or
   launch-count effect.

3. **ioctl profile is identical host vs guest.** Both issue the same ~985
   ioctls over a 16-token run (~20/token steady-state), ~70% RM_CONTROL (0x2a).
   So libcuda's control-command volume is intrinsic — NOT a guest-specific slow
   fallback. The gap is *per-ioctl latency*, not count.

4. **Per-ioctl overhead ≈ 880 µs.** 37 ms/token guest overhead ÷ ~42 ioctls ≈
   880 µs extra per ioctl vs host.

5. **The QEMU↔isolate socketpair is NOT the bottleneck.** Standalone ping-pong
   (`/tmp/sp_pingpong.c`, host, 11 cores):
   - cross-core busy-poll: **~10 µs** RTT (stable under scheduler load)
   - cross-core blocking: ~30–35 µs
   - same-core blocking, oversubscribed: ~123 µs
   Eliminating the socketpair wakeup saves ~20 µs/ioctl ≈ **~2% of the gap**.

## Conclusion / bottleneck hypothesis

The dominant cost (~850 µs of the ~880 µs/ioctl) is **the guest↔host crossing**,
not the host-side socketpair: the guest task blocks on `wait_for_completion` →
the **vCPU deschedules/idles → completion IRQ injection → vCPU wakeup**. In a VM
this round-trip is far costlier than a host socketpair wakeup (the ping-pong was
host-only, no VM transitions).

**Implication for the command buffer:** it pays off ONLY via its **guest-side
spin** on shared completion memory (no vCPU deschedule, no IRQ wakeup). A
QEMU↔isolate-only fast-path buys ~2% and is not worth building. The guest-side
half is the prize.

## Experiment 2: guest-side completion spin — NEGATIVE (vCPU-wakeup ruled out)

Made the guest module bounded-spin (~600 µs budget, `try_wait_for_completion` +
`cpu_relax`) on the forwarded-ioctl completion before blocking. Result: decode
**unchanged at ~28 t/s** (same as the non-spin baseline; the original "24" was
pre-security-fix noise). So keeping the vCPU running (no idle→IRQ→wakeup) buys
**nothing** → the guest-side vCPU wakeup is **NOT** the bottleneck.

## Reframed bottleneck (after ruling out socketpair AND guest-wakeup)

The ~1.6 ms/ioctl overhead (≈37 ms/token ÷ ~20 ioctls) is on the **host side**,
in QEMU's machinery between receiving the virtqueue kick and producing the
response — candidates, none yet isolated:
- thread-pool dispatch latency (`thread_pool_submit_aio`) getting the ioctl work
  to a pool thread under load;
- the reader-thread → worker pthread_cond handoff for the response;
- the completion callback (`nvkvm_ioctl_work_done`) running under the **BQL** —
  if vCPUs hold the BQL, the completion is delayed (ms-scale under contention);
- per-ioctl serialization on QEMU/stub locks.

A spin on the host side wasn't tested; the **command buffer (guest↔isolate,
bypassing QEMU entirely)** removes ALL of the above — the isolate's spin-thread
does recv→ioctl→send inline with no thread-pool, no cond handoff, no BQL. So the
command buffer remains the likely fix, now for a host-QEMU reason rather than a
guest-wakeup one.

## Experiment 3: host-side stage breakdown — LOCALIZED (the round-trip)

QEMU timestamps around the three host-side stages, averaged over decode:

```
avg_us   dispatch = 59.1   work = 883.8   complete = 52.8
```

- **dispatch** (kick → pool thread starts): 59 µs — minor
- **complete** (worker exit → BQL completion callback): 53 µs — minor
- **work** (`nvkvm_req_ioctl_on_isolate`: send to stub → response received):
  **884 µs — 89% of the per-ioctl cost.**

The standalone socketpair is ~30 µs and the host ioctl ~7 µs, so the 884 µs is
neither the wire nor the driver — it is the **chain of sleeping-thread wakeups**
in the round-trip:

```
QEMU worker --send--> stub RECEIVER (recvmsg wakeup) --enqueue--> stub WORKER
  (queue_cond wakeup) --ioctl--> stub WORKER --send--> QEMU READER
  (recvmsg wakeup) --signal--> QEMU WORKER (pthread_cond wakeup) --> response
```

~4 sleeping-thread wakeups, ~220 µs each under decode's scheduler contention
(consistent with the same-core-loaded ping-pong ~123 µs). ~36 ioctls/token ×
~884 µs ≈ 32 ms/token = the bulk of the 36 ms/token decode time.

## Verdict: the command buffer is justified (and is the only clean fix)

All three non-round-trip suspects are minor (socketpair ~2%, guest-wakeup 0,
dispatch+complete ~110 µs). The cost is the **four-thread-wakeup round-trip**,
and the only thing that removes all four at once is replacing trap-and-defer
with **poll-and-handle-inline** — the command buffer: the isolate's spin-thread
reads the command inline, runs the ioctl inline, writes the result back inline;
the guest reads it inline. No socketpair, no receiver/worker/reader handoffs.
`work` collapses from 884 µs toward the ~10 µs floor (host ioctl + cache
transfer) → decode approaches host rate (~368 t/s); ~15× for single-stream.
Piecemeal busy-poll of the four handoffs would burn cores, capture only part,
and approach the command buffer's complexity anyway — so it is NOT preferable.

## Command-buffer build plan (next milestone, multi-week)

1. **Per-isolate shared command ring** — host mints a memfd, maps it into the
   guest GPA (like other GPU mmaps) AND passes it to the isolate via SCM_RIGHTS;
   both map it. Header = ring indices + per-slot {cmd, sizes, txn, status}.
2. **Fast-path classification** — only ioctls the isolate fully services without
   QEMU (no KVM memslot install, no fd creation): RM_CONTROL (no embedded fd),
   RM_FREE, query controls. Everything else stays on the virtqueue path.
   Default-deny.
3. **Isolate spin-thread** (per isolate) — polls the ring; on a command:
   copy-in to private buffer (TOCTOU, audit P2-2), run gates, `stub_ioctl`,
   write result + status back, advance the ring. Adaptive: spin while busy,
   block (futex on a ring doorbell) when idle.
4. **Guest side** — write command, then adaptive spin (bounded) on the result
   slot, fall back to a virtqueue SYNC ("wake me on completion") to sleep
   without burning a vCPU. Lost-wakeup-safe (futex compare-value).
5. **Interrupt path** — mirror SIGUSR1: mark the ring slot + signal the isolate
   spin-thread, guarded like the current model.
6. **Security invariants** — copy-in before check (P2-2), treat ring head/tail
   as hostile (bound every index), per-isolate ring (no cross-tenant), never
   expose privileged pages.

Ceiling ≈ host rate (the isolate still runs ~36 real ioctls/token). Adaptive
spin needed so idle/many-tenant cases don't burn cores.

## Probable fixes (ranked by expected payoff, pending the next experiment)

1. **Guest-side completion spin** (bounded, then block) — eliminate the vCPU
   deschedule/IRQ-wakeup on each forwarded ioctl. Likely the bulk of the prize.
   Must be adaptive (spin tens of µs → futex/block) to avoid burning a vCPU when
   idle, and respect the security model (no privileged pages exposed).
2. **Shared-mem command buffer** (guest↔isolate, both spin on separate cores) —
   the full version: removes socketpair + VM-exits + wakeups for pure ioctls.
   Floor ≈ host rate (≈368 t/s), since the host still executes ~42 ioctls/token.
   See `fastpath_command_buffer` notes; needs copy-in TOCTOU discipline (cf.
   audit P2-2), per-isolate spinloop, lost-wakeup-safe SYNC.
3. **Adaptive stub mutex** (spin→futex) — modest; stability/groundwork. The
   current `fs_mutex` is a correct non-adaptive Drepper futex mutex.
4. Socketpair→shared-ring busy-poll (QEMU↔isolate) — DEPRIORITIZED (~2%).

## Why this is the floor without SR-IOV

SR-IOV is faster because hardware VF isolation lets the guest ring the GPU
doorbell directly with no mediation. On a commodity (non-SR-IOV) GPU the isolate
MUST execute each forwarded ioctl to enforce the security boundary — that
host-ioctl-per-command is the irreducible price of software isolation on a
shared GPU, and exactly what lets nvkvm run where SR-IOV cannot.
