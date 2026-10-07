# Command Buffer — SPSC-Ring Fast Path (build spec)

Status: SPEC (2026-05-31). Supersedes the linked-list sketch in
`perf_findings.md`. Rationale and measurements: `perf_findings.md`,
memory `decode_ioctl_profile`.

## Why

Decode is ~884 µs/ioctl, and 89% of that is the **four sleeping-thread wakeups**
in the QEMU↔isolate round-trip (stub receiver, stub worker, QEMU reader, QEMU
worker), NOT the wire (~30 µs) or the host ioctl (~7 µs). The fix is to replace
trap-and-defer with **poll-and-handle-inline**: a per-isolate shared-memory ring
the isolate spin-thread reads directly, runs the ioctl inline, and writes back —
no socketpair, no thread handoffs. `work` collapses toward the ~10 µs floor →
decode approaches host rate (~368 t/s; ~15× single-stream). Ceiling = host rate
(the isolate still runs the same ~36 ioctls/token).

## Core principle: the ring is a TRANSPORT, never a hold buffer

The isolate consumer reads a command and **immediately copies it into private
memory**, freeing the ring slot in µs. The (possibly long) ioctl then executes
from the private copy. Consequences:
- **No head-of-line blocking by slow ioctls** — a 3 s ioctl does not occupy a
  ring slot for 3 s; the slot is freed on copy-out.
- **No in-flight tampering** — the isolate never re-reads guest-writable memory
  mid-operation (this is audit P2-2 made structural).
So *every* isolate-serviceable ioctl rides the ring regardless of its duration.

## Model

Two **SPSC byte-rings per isolate** + the existing **virtqueue** as control
channel:
- **request ring**: producer = guest (untrusted), consumer = isolate (trusted)
- **response ring**: producer = isolate (trusted), consumer = guest (untrusted)
- **virtqueue** keeps: ring setup, **grow/resize** handshake, **sync/doorbell**,
  and the ioctls the isolate cannot service alone (see classification).

SPSC needs **no mutex, no double-guard, no ABA reasoning**: producer owns `tail`,
consumer owns `head`; each only acquire-reads the other's counter, and a stale
read is always conservatively safe.

### Ring mechanics
- Region size `N` (pow2). Two cache-line-separated **free-running** counters
  `head`/`tail` (empty: `head==tail`; full: `tail-head==N` — unambiguous).
- Records are length-prefixed and aligned: `[u32 len][u32 type][payload]`,
  payload = ioctl request/response struct + data + **txn_id**. txn tracking
  sits one level above blocks, exactly as before.
- **Pad-to-end on wrap**: if a record would straddle `N`, emit a skip record so
  every record is contiguous (no split reads).
- Producer: free-space check (acquire other's counter) → write payload →
  **release-store** own counter `+= len`. **Doesn't fit (ring full, or record
  > N) ⇒ fall back to the virtqueue transaction for that one ioctl** — the
  virtqueue is always the correct path, so the ring is a pure optimization and
  the producer NEVER blocks on a full ring (no producer-stall deadlock, and
  oversized/rare ioctls naturally take the slow path they'd want anyway).
- Consumer: acquire other's counter; while not empty: read `len` at `head%N`,
  **bounds-check** (`len ≤ available`, `≤ N`, aligned) → **copy record to
  private memory** → validate (fast-path allowlist) → execute → **release-store
  `head += len`**.

## Isolate consumer loop (the decode fast path)

```
spin:
  while ring not empty:
     copy record out (private); bounds-checked
     validate command (per-cmd allowlist)
     if fast-class:  run stub_ioctl INLINE, write response to response ring
     else (slow):    hand the private copy to a worker; worker runs it and
                     writes the response when done   # ring keeps draining
  adaptive: spin a budget; if still empty, sleep (see sync/doorbell)
```

- **Fast ⇒ inline** = no thread handoff = the win (decode's RM_CONTROLs, ~7 µs).
- **Slow ⇒ worker** = the (now-negligible relative to a long op) handoff, so a
  slow isolate-serviceable ioctl (e.g. a blocking event-wait) never stalls the
  ring reader. The fast/slow split lives HERE (inline vs worker), fed by the
  per-cmd latency measurement — it is NOT a ring-vs-virtqueue split.

## Wait / wake — virtqueue lifecycle, NO futex (Phase 3)

The consumer must NOT burn a core when the guest is idle (explicit constraint),
**and** we want one wait primitive, not a futex that can't be combined with the
isolate's `recvmsg`. So the blocking lives **entirely on the guest side**, over
the virtqueue, and the isolate's idle state is just its existing blocking
`recvmsg`. There is **no shared synchronisation word in guest memory** — a
security win over a futex flag (which the guest could scribble; DoS-only, but
unnecessary surface).

Two long-running virtqueue calls, **at most one of each in flight per isolate**:

1. **`enter_loop`** — tells the isolate to enter its ring consumer loop; the
   virtqueue transaction **stays in flight until the isolate EXITS the loop**
   (after X idle cycles, ~hundreds of µs), and completes returning the
   **last-processed ring txn id**. The guest issues this once when it starts a
   busy phase; it stays in flight across the whole phase, so the hot path is
   **pure ring, zero per-record virtqueue cost**.
2. **`wait_on_ring`** — blocks the guest until one ring response completes (like
   poll), or returns immediately if everything published is already done.

Isolate threading (one thread, no futex, no spinner thread):
```
idle:  blocking recvmsg(socket)                       # 0% CPU, the existing state
on enter_loop:
  loop:
     drain request ring: inline-fast → response ring; slow → worker
     ring EMPTY → check socket once (MSG_DONTWAIT); dispatch any normal cmd
     ring idle X cycles AND socket idle → COMMIT-TO-EXIT (see invariant)
  complete enter_loop(last_processed_txn)              # back to blocking recvmsg
```
Only one socket syscall at the *drain edge* (not per spin iteration). During
active decode the loop spins continuously (right latency tradeoff while the GPU
is busy); when truly idle it exits → blocking `recvmsg` → 0% CPU.

### The exit-edge invariant (this is the critical section)

`enter_loop`'s lost-wakeup-freedom rests on two orderings — the direct analog
of a futex presleep re-check, carried over the virtqueue completion instead of a
shared word:

- **Consumer:** after deciding to exit (X idle cycles), mark "exiting" then
  **RE-CHECK the request ring** (`nvkvm_ring_has_work`, an acquire-load of
  `tail` vs `head`). If a record raced in, **abort the exit** and keep looping.
  Complete `enter_loop` only if the ring is empty *at this commit point*.
- **Guest:** order **publish T (release-store tail) → THEN check whether
  `enter_loop` is still in flight.** On completion with `last_processed < T`,
  **re-enter** (issue a fresh `enter_loop`).

Every interleaving is then safe: either the consumer sees T on its final
re-check (stays), or it has already exited completing with `last < T` (guest
re-enters). The dangerous middle — guest sees "in flight" while the consumer has
silently exited past T — is foreclosed by the consumer's final re-check. Omit
the re-check and the race returns. This replaces the earlier single `sync`
command (which conflated "keep the consumer alive" with "wait for progress").

For decode this is clean because the **entire hot stream is fast/inline** (see
Classification): the consumer loop itself produces every response, so
"loop exited" ⟺ "all responses done" ⟺ `wait_on_ring` reports done. The only
wrinkle is a *slow* ring op handed to a worker — its response can land after the
loop exits, so the worker must also be able to satisfy `wait_on_ring`; decode
has none, so build the inline-only model first.

(Superseded design — kept for history: an in-memory futex on a guest-shared
`consumer_sleeping` word with guest-kick→QEMU-`FUTEX_WAKE`. Dropped because it
needs a second thread + a guest-writable sync word and can't share the
isolate's `recvmsg` wait. The lost-wakeup analysis carried over to the
exit-edge re-check above.)

## Classification — what rides the ring

Ring = ioctls the isolate fully services with **no QEMU/KVM/fd mediation** and
(for inline) **bounded-fast**:
- **Ring**: RM_CONTROL (no-embedded-fd subset), RM_FREE (0x29), simple
  (non-memory, non-event) RM_ALLOC (0x2b). [≈ the 70% RM_CONTROL + frees +
  simple allocs of decode traffic]
- **Virtqueue (must)**: anything needing QEMU/KVM — RM_MAP_MEMORY (0x4e),
  RM_ALLOC_MEMORY/OS_DESCRIPTOR (0x27), MAP_MEMORY_DMA, UNMAP_*, **all UVM**
  (mm-bound), OPEN_DEVICE, REGISTER_FD, GET_PID_INFO (init-ns answer),
  EXPORT_OBJECT_TO_FD, ALLOC_OS_EVENT (embedded fd), NVKMS/DRM (graphics),
  plus control/grow/sync.
- The exact RM_CONTROL `cmd` subset (and inline-vs-worker per cmd) is set by the
  **per-cmd latency measurement** (instrument the stub around `stub_ioctl`,
  record count + avg/max latency per NVOS54 `cmd`; any control with high max or
  an embedded fd → off-ring or worker; the rest → ring/inline). Tightens
  `nvkvm_ctrl_allowlist.h`.

  **MEASURED 2026-05-31** (cmd-distribution probe over a decode run, 679
  RM_CONTROLs / 64 tokens): the hot path is ~6 control cmds repeating ~once per
  token — all `NV2080` subdevice + channel (`906f`/`c36f`) controls:
  `0x2080a084 0x2080a026 0x20809064 0x20809009 0x20809001` (92× each),
  `0x20802209` (46×), then `0x00000d01 0x906f0101 0x20801303 0xc36f0108
  0x0080170d 0x20801218` (tail). All are fast scheduling/fence/query controls —
  isolate-serviceable, no QEMU/fd, non-blocking → **the entire decode
  RM_CONTROL stream is ring + inline**; no slow control on the hot path, so the
  inline-vs-worker split doesn't bite decode. The ring control-allowlist =
  this observed set (a subset of the existing ctrl allowlist). The stub latency
  measurement remains only to catch a rare slow control before it rides inline.

## Guest-side flow (puts it together)

- Busy phase: issue `enter_loop` (async, stays in flight) → publish requests to
  the request ring + read responses off the response ring in real time (pure
  ring, no virtqueue — the hot path). Keep `enter_loop` in flight the whole time.
- If `enter_loop` completes (isolate hit its idle timeout) while the guest still
  has unprocessed txns (`last_processed < last_published`), re-enter — per the
  exit-edge invariant this never drops a record.
- Drain/idle: when the guest has published everything and only awaits the last
  responses, call `wait_on_ring` to block its vCPU (KVM deschedules it) until a
  response lands or all published txns are done. This is the only place the
  guest sleeps, and it is off the hot path.

## Security invariants

- **Copy-out before validate/use** (P2-2): never act on guest-writable memory.
- **Bounds-assert every access** (offset, `ptr+size`) to the ring extent; on
  violation the offending guest's **isolate is torn down and its ioctls error**
  (per-guest containment = equivalent to a guest SIGKILL; never cross-tenant,
  never the host). Corruption by an untrusted producer = **DoS-only**.
- **Forward progress / no infinite loop**: bound the consumer's walk to
  `N / min_record` per pass; `sched_yield` if a race-reloop spins too long.
- **acquire/release** on the `head`/`tail` counters and record publish.
- **No guest-writable sync word**: the wait/wake lifecycle is owned by the
  trusted isolate + QEMU (`enter_loop`/`wait_on_ring`); the guest influences it
  only by producing records and issuing the (permitted) virtqueue calls.
- **Per-isolate** rings → a stalled/abused ring hurts only that guest; its
  virtqueue path keeps working.
- Offsets are **in-buffer**, never global pointers.

## Build phases

1. **Ring primitive + tests** (host-only unit test: SPSC, wrap, bounds, hostile
   fuzz, the `enter_loop` exit-edge lost-wakeup soak). No GPU.
2. **Per-isolate ring setup over the virtqueue** (memfd mint → GPA install +
   SCM_RIGHTS to isolate; both map) + the grow handshake. *(SETUP_RING + probe
   DONE and **HW-VALIDATED** 2026-05-31 on vast.ai RTX 3060 / 580.159.04: the
   handshake fires at every isolate spawn against the real hardened+seccomp'd
   stub — "ring N ready ... bidirectional probe OK", 10/10 success; single +
   4× concurrent matmul still PASS = no data-path regression. GPA install into
   the guest deferred to Phase 4 where the guest mapping consumes it.)*
3. **Isolate consumer loop** (driven by `enter_loop`): ring read → copy-out →
   validate → inline ioctl → response ring; the exit-edge re-check + idle
   timeout; one thread also polls the socket at the drain edge. *(Exit-edge
   concurrency core BUILT + TSan-proven: `nvkvm_ring_has_work` +
   `ring_loop_test.c` — the consumer loop lifecycle + guest re-enter, lost-
   wakeup-free. The remaining wiring — route reads into the real `stub_ioctl`,
   and the `enter_loop`/`wait_on_ring` virtqueue calls + their QEMU async
   completion — is GPU/guest-coupled and lands with Phase 4.)*
4. **Guest side**: ring producer/consumer + the `enter_loop`/`wait_on_ring`
   lifecycle (re-enter on `last_processed < last_published`); route fast-class
   ioctls to the ring, the rest to the virtqueue.
5. **Inline-vs-worker** dispatch in the isolate for slow isolate-serviceable
   ioctls; wire the per-cmd classification.
6. Measure decode end-to-end; expect `work` → ~10 µs, decode → toward host rate.

## Future (Phase 7, measure-gated): a guest↔QEMU ring for QEMU-only ops

After the isolate ring lands, the residual decode overhead is the QEMU-only ops
(some UVM / bookkeeping) still on the virtqueue (~a few per token). A second ring
*instance* — guest↔QEMU, QEMU runs a spin-thread polling it — would bring the
**pure-QEMU-bookkeeping** subset to ~µs too, reusing this exact ring primitive
(no new concurrency code). DO NOT build speculatively: only if measurement shows
QEMU-only ops are the residual bottleneck after Phases 2–6.
- Caveat (precise): only the ops that install a **KVM memslot**
  (`KVM_SET_USER_MEMORY_REGION`) are genuine main-loop/BQL work and must stay on
  the virtqueue. But memslot install is **window setup, not per-mmap** — the KVM
  user-memory region is a single big GPA *window*, not one slot per mapping.
  The common path — `mmap`-ing a GPA into the VMM's VA inside the
  already-installed window (a `MAP_FIXED` slice) — is just an `mmap` syscall:
  fast, no BQL, and the ring tolerates its (rare) blocking. So those per-mmap
  UVM/RM ops **can** ride the QEMU ring; only the infrequent window-grow /
  memslot-install stays on the virtqueue. The QEMU-ring helps the no-memslot
  QEMU-only subset, which includes most mmaps.
- Net transport lines per guest then: virtqueue (control/setup/relief/memslot) +
  isolate ring (isolate-serviceable ops) + QEMU ring (pure-QEMU fast ops).

## Open items

- Run the per-cmd latency measurement to lock the ring control-cmd allowlist +
  inline-vs-worker thresholds (next step).
- Confirm which isolate-serviceable ioctls are genuinely slow (event-waits) so
  they take the worker path.
- Grow policy: the ring is a fixed mmap; **resize only at quiescence** (no
  in-flight ring txns). Mechanism: the producer notices chronic
  fall-back-to-virtqueue (ring too small), requests a grow over the control
  channel; the consumer drains to empty (sync confirms `head==tail`), both sides
  swap to a larger memfd, resume. No need to resize under load — fall-back to
  the virtqueue absorbs bursts. So resize is an optional throughput tweak, not a
  correctness requirement.

---

## Measured results (2026-05-31, vast.ai RTX 3060, driver 580.159.04)

The full data path is built and HW-validated end-to-end (stub consumer loop +
QEMU `enter_loop` offload + guest pump/producer):

- **Correctness**: 1024² fp32 matmul byte-exact through the ring (exec=36,
  punt=1 for a single run); 4× concurrent matmul all PASS; cuInit PASS; LLM
  decode runs clean. Zero regression vs the virtqueue path.
- **Ring is actually used**: during LLM decode the ring carries the bulk of the
  hot controls — e.g. **exec=1446, punt=54** over ~120 tokens (~12 flat
  RM_CONTROLs/token), with the slow path seeing only ~40 ioctls total (setup).
- **Batching works**: `enter_loop` fired <100 times for ~482 ring controls in a
  decode (≈5–10+ controls per round-trip) — the stub stays spinning across a
  burst, so per-control cost is a memory write + a µs-scale busy-wait, not a
  full virtqueue round-trip.

**But it does not improve decode throughput.** A/B with
`ring_enable={0,1}`: **~28 t/s either way** (within run-to-run noise). The
reason: control-RTT is only ~1–2 % of per-token wall-clock. At ~28 t/s a token
is ~35 ms; ~12 controls × ~45 µs saved ≈ 0.5 ms ≈ 1.5 %. The decode bottleneck
is GPU compute plus the **launch/sync path, which is not ioctl-bound** — a
`gpu_bench` noop-launch loop issues *zero* per-launch RM_CONTROLs (work is
submitted via the mapped pushbuffer/doorbell and completion is polled on a
mapped fence, all direct memory, no forwarded ioctl). So the original premise
(decode is dominated by control-RTT *volume*) does not hold for these workloads.

**Cost.** Keeping the isolate spinning during a burst (and the guest producer
busy-waiting for its response) trades host/guest CPU for control latency. Since
latency is not the bottleneck here, that trade is a net negative for
compute/decode workloads — hence **`ring_enable` defaults OFF**.

**When it would pay off** (left enabled-on-demand via the param): workloads that
*are* control-latency-bound — very high token-rate tiny models, speculative
decode, or any path where a synchronous burst of small RM_CONTROLs sits on the
critical path. The transport is also a reusable building block (a guest↔QEMU
control ring for mmap-class ops was always a separate, planned use).

**Not pursued here** (the real decode optimisation): the launch/sync path is
mapped-memory, not ioctl — so accelerating it is a different effort (doorbell/
fence trap behaviour, vCPU scheduling), not a command-forwarding problem.

### Batching validated (the virtqueue-txn test)

Follow-up A/B (runtime `ring_enable`/`ring_idle_us` toggles, no module reload):

- **Steady-state decode**: ~**29 ring controls per `enter_loop`** (window:
  ring_exec +147, enter_loop +5) — the stub stays spinning across a token's
  control burst, so 29 control-ioctls collapse to ONE virtqueue round-trip.
- **Cumulative incl. setup**: ring_exec=2000 / enter_loop=531 / slow_fwd=2823 →
  total guest→QEMU virtqueue txns ≈ 3354 with the ring vs ≈ 4823 without
  (~30 % fewer; the 2000 ring controls alone drop from 2000 txns to 531).
- **Throughput unchanged anyway**: ~28 t/s ring on or off.

This is the decisive check: virtqueue transactions drop ~29× for ring-routed
controls (and ~30 % overall), yet throughput does not move — confirming the
control-forwarding path is genuinely not the decode bottleneck (it's GPU compute
+ the mapped doorbell/fence launch path). Stability: matmul ×3, 4× concurrent
matmul, and repeated decode all PASS with the ring on (no `rmmod` cycling). The
earlier VM hang was a test-harness artifact — an absurd `ring_idle_us` (~50 s
stub spin) blocking the pump's `kthread_stop`, compounded by `rmmod`+`insmod`
(forbidden per the iteration model). Fixed by capping the stub idle budget
(`NVKVM_RING_IDLE_MAX`) so no parameter value can wedge guest teardown.
