# The slow-op discipline audit — the async lane is built, correct, and has no driver

**STATUS: ANSWERED 2026-09-06 (w383) — the lane has a driver. `docs/design/w383_the_doorbell_is_a_schedule.md`.**
Audit against `nvkvm-pv` as the fast/slow oracle, at the owner's suggestion. ⊘ Findings only;
no remedy applied *in this document*. Read with
`ownership_gpga_leases_and_the_two_channel_kinds.md` (the same night's ruling).

> ### ✔ F1, F2 and F2b ARE CLOSED. F3, F4, F5, F6 ARE NOT.
> `[measured w383, real GA106, cup3]` `TRAPWITNESS off_trap_claims=0 inline_exceptions=856`
> on the control became **`off_trap_claims=2812 inline_exceptions=35`** with the lane armed,
> on a boot that still returned `CUP3_VAL=43` with zero host Xids. §0's headline — *"we have
> the right shape and nothing runs it"* — is no longer true: `pubqueue` has a producer
> (`SharedDoorbell::ring`), a consumer (`doorbell_publish_loop`) and a thread
> (`kayfabe-doorbell-publish`), started in `attach_ram` and joined in `detach_ram`.
>
> ⊘ **F6 got WORSE in one respect and it is named rather than left to be found**: there are
> now **two** off-trap threads relying on the same unwritten RCU/`memory_region_ref` argument,
> and still only two `TrapGuard::enter()` sites.
>
> ⊘⊘ And the audit's own §4 item 1 — *"the shape exists and is good"* — was **half right**.
> The plan/execute/revalidate shape was fine; the queue's **coalescing** was not, and it took
> a hardware boot to find out. See `publication_off_the_bql.md`'s STATUS block.

---

## 0. ★★★★★ THE HEADLINE: WE HAVE THE RIGHT SHAPE AND NOTHING RUNS IT

`kayfabe-device/src/pubqueue.rs` — the deferred publication lane — has **zero production
producers and zero production consumers.** `grep -rn 'pubqueue' crates/ --include=*.rs` returns
8 hits: one `pub mod`, four comments, two compile-fail tests, and the module. Second spelling
(`PublicationQueue` / `take_blocking` / `PublicationWorker`) reaches only the module and one UI
test. **`take_blocking` has no production caller.**

Corroborated from the other end: `thread::spawn|thread::Builder` across the VMM-side production
crates yields **exactly one** off-trap thread — `shim.rs:11908`, `"kayfabe-completion-observer"`.
`pubqueue.rs:501` and `reclaimtick.rs:236` are both under `#[cfg(test)]`.

⇒ **There is no publication worker thread.** The mint-site rationale that reads *"once the
publication lane is armed the same line runs on the worker"* describes a state that does not
exist.

## 1. The oracle — what nvkvm-pv measured (Phase 1)

The load-bearing row, **MEASURED**, `tests/perf/README.md:49`:
> `alloc+free RTT : host 133 / guest 3863 us = **29x** (tripwire: <50x — KNOWN tax, the
> standing optimization target; never been at parity)`

⇒ **RM alloc/free is the slowest forwarded operation in a shipped implementation that is
otherwise at ~99 % of native.** Everything kayfabe's `RmBackend` verb surface does is that
operation.

Other measured slow rows: OS_DESCRIPTOR registration `2 GiB | 5.7 s`; managed-alloc cycle
14–19 ms; synchronous readback under the BQL — *"the guest's vCPUs are stopped for the whole
transfer … the cost is not the bandwidth, it is stopping the VM to pay it."*

★★ **Independent corroboration, two codebases:** kayfabe measures its worst single RM disposal
at **≈3.3 ms** (`[measured w317c1diag]`); nvkvm-pv measures guest alloc+free RTT at **3.863 ms**.
Same order of magnitude for one RM round trip, different harnesses. **The oracle's
classification transfers.**

## 2. Findings, ranked

| # | severity | site | slow op | consequence |
|---|---|---|---|---|
| **F1** | worst — vCPU under BQL | `shim.rs:9400` in `publish_vas_rows` ← `SharedDoorbell::ring` `:4877` | per-chunk guest-RAM pin, in a loop | whole-VM freeze; our own worst trap is **2.88 s** |
| **F2** | worst — vCPU under BQL | `kayfabe-rt/src/device.rs:2032` | **every** host RM verb | always takes the `inline_under_bql` branch |
| **F2b** | structural cause | `pubqueue.rs` | — | **the lane has no producer, no consumer, no thread** |
| **F3** | high | `device.rs:563` `self.returned.wait(g)` | unbounded condvar park | vCPU parks **under the BQL**, where no other vCPU can run to release it |
| **F4** | medium | `shim.rs:3879` in `MappedFb::write` | staged host RM frees | a guest **framebuffer write** pays up to 53 ms |
| **F5** | medium — main thread | `kayfabe-core/src/gpu.rs:2115` `Drop for Proc` | staged-release verb chain | also reached from QEMU's main thread via `kayfabe_shim_unrealize` |
| **F6** | instrument gap | only 2 `TrapGuard::enter()` sites | — | see §3 |

★ The tree **says F1 itself**, verbatim at `shim.rs:9240-9246`: *"no thread but the trapping vCPU
could ever run a publication … it is why publication still runs inline with the BQL held"*, and
at `:9258`: *"⚠ This lift alone moves nothing off the BQL."*

## 3. ⚠ F6 — the instrument's blind spot CORRELATES with the hazard

Only **two** `TrapGuard::enter()` sites exist, both BAR MMIO (`shim_unsafe.rs:1299`, `:1341`).
The other **17** exported C entry points install none — so a host verb beneath `realize`,
`unrealize`, `region_add`, `install_window` … mints a clean `OffTrap::claim` and is **counted as
off-trap**. `kayfabe_shim_unrealize` (`:886`) is main-thread teardown and reaches F5's verb
chain: invisible to the census **by construction**.

⇒ The clause-(a)/(b) census cannot distinguish *"no violation"* from *"not instrumented"* on any
path but BAR MMIO. Same shape as everything else this project has paid for.

## 4. ★ What is ALREADY CORRECT — the pattern to copy, not invent

1. **Two-phase plan/execute with revalidation — `SharedDevice::verb_op`** (`device.rs:2030`):
   plan under the lock, **drop the lock**, run host verbs lock-free, and on staleness re-enter
   from the top with full revalidation, bounded by `MAX_COMMIT_RETRIES = 8`. `RingPlanned` does
   the same for the ring. ★★★ **This is exactly the async+revalidate discipline the audit asks
   for. It is built. F2 is that nothing drives it from a worker.**
2. **Asymmetric lock API making a vCPU block unrepresentable — `ReclaimTick`**: the trap side
   reaches only `try_lock`; there is *no* blocking method a trap can call. Enforcement by type,
   not by review.
3. `pubqueue::offer` does no I/O under the guard; `notify_one` is deliberately after the drop.
4. `DirtyGate::published` (w318) — build the value *before* the lock, after a real inversion.

⇒ **The remedy is not a redesign.** The shape exists and is good; what is missing is the thread
that runs it, and the honesty that F1/F2 are inline today.
