# The ten goals — measured status

**STATUS: LIVE, 2026-09-13 (w619).** Every row cites a measurement from a boot on a **matching**
GA106 (RTX 3060, driver 580.159.04) that graded the raw client `(P)`. ⊘ A row with no measurement
says so; *"not started"* and *"believed fine"* are different states and are not merged.

## The table

| # | goal | state | the measurement |
|---|---|---|---|
| 1 | Blackwell boots | **not started** | — |
| 2 | zero read traps; write traps only in BAR0, not PRAMIN, not BAR1/2 | **one page short** | `pages_touched=1`, all the counter |
| 3 | no blocking calls or held locks on the vCPU | **met, with one caveat** | `inline_exceptions=0` every boot; `VCPU-BLOCKING none`; rank-0 `worst_wait=0us slow_waits=0` |
| 4 | the DoorbellTable wired | **not started** | `dbtable.rs` has ZERO callers; it replaces a 630-line path reaching 14 subsystems |
| 5 | code rot cleaned or marked | **advanced** | three censuses adjudicated (w605), 16 unrecoverable boot tags grandfathered, full workspace gate green |
| 6 | every write trap sub-millisecond | **one trap over** | `slow_traps(>1ms)=1`, `worst_trap≈17ms at bar0+0x110c00` |
| 7 | raw client passing the mean test | **MET** | `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8 verified`, `MEAN_FALSIFIER=PASS`, on every boot this session |
| 8 | TWO raw clients in parallel (needs epoll in workers) | **not started** | — |
| 9 | the LLM working, then at parity | **not started this session** | last known: `the_llm_fails_on_all_three_doorbell_arms` |
| 10 | more CUDA apps, then all of it on Blackwell | **not started** | — |

## ★★★★★ GOAL 2, RE-GRADED AT HEAD (w632) — one page touched, and it is the counter

`[measured w631a, rev 9728dbc9, a boot that graded `(P)`]` — **nine commits after the BAR work
was first measured**, because a result that old is a claim about a tree that no longer exists:

    BAR1 (translated)   0 reads / 0 writes
    BAR2 (translated)   0 reads / 0 writes
    PRAMIN-ONLY         0 reads / 0 writes
    BAR0-READ-HOTSPOTS  pages_touched=1  reads_from_live_pages=129
                        reads_from_BACKED_pages=0   top[+0xbb0000=129]
    W392D_GUEST_OUTCOME=(P)

⇒ **`pages_touched=1`.** One page on the entire device produces a read trap, it is the
free-running counter, and `reads_from_BACKED_pages=0` says nothing leaked out of a page the cut
is supposed to serve.

## Goal 2, in detail — the one that moved

| surface | reads | writes |
|---|---|---|
| BAR0 excluding the counter page | **0** | traps — the control plane, which is allowed |
| the free-running counter (`+0xbb0000`) | **~132** ⊘ the only read traps left | — |
| PRAMIN | **0** | **0** |
| BAR1 | ~0–55 | ~2 300–3 500 |
| BAR2 | ~2 | ~1 450–1 600 |

★ **BAR0's read surface went 184 585 → ~134** and every remaining read is the counter page.
★ **PRAMIN is at zero in both directions, proven against its own control** — `KAYFABE_PRAMIN_SLOT=0`
measures 22 / 67 956, the default measures 0 / 0, same binary, both arms `(P)`.

### What goal 2 still owes, and what is known about each

**The counter page.** It cannot be shadowed — it changes continuously — so the only answer is to
map the HOST's own usermode page read-only. `the_counter_page_and_the_device_view.md` carries the
design, and its containment is **measured**: an `O_RDONLY` device node refuses a writable `mmap`
with `EACCES` and refuses `mprotect` back to writable with the same errno, both unprivileged.
⊘ The wire verb that ships the descriptor to the VMM is **not built**.

**BAR1/BAR2.** `[measured w608]` 184 distinct pages cost 3 952 trapped accesses — 21 per page —
because fills are on demand and the guest re-touches a page before its slot lands. 89.2 % of that
working set (100 % of BAR1's) is needed only after the first channel birth, so the owner's
map-at-create ruling is aimed at the right traffic.
⊘⊘ **The first implementation FAILED** (w618): enumerating BAR1's leaves mapped 253 pages the
guest never touches, made `ALREADY-COVERED-EARLY` 4.5× worse, and did not reduce BAR1's traps at
all. Default off behind `KAYFABE_PREMAP_BAR1=1`. **Why premapping did not reduce the traps is
unexplained** and is the open question.

## Goal 6's one trap, and why it is not excused

`worst_trap≈17 ms at bar0+0x110c00` (the GSP RPC submit), once per boot. The owner's rule excuses
unscheduled time only when it is a vCPU steal. `[measured w593, two boots]`:

    wall 18 331 us   thread_cpu 18 098 us   off_cpu   233 us
    wall 16 956 us   thread_cpu 16 771 us   off_cpu   185 us

⇒ **98.9 % of it is the thread RUNNING.** It is our own work, the excuse does not apply, and the
site is one-time by shape (`slow_traps=1` while 359 doorbells and 1 181 invalidates pass cleanly).

## Goal 3's caveat

`inline_exceptions=0` and every lock census is clean, so nothing BLOCKS on the vCPU. ⊘ But goal 6's
17 ms is CPU burned inside an MMIO exit, which is the vCPU stopped for 17 ms without blocking on
anything. The two goals disagree about whether that is a violation; goal 6 is the one it fails.


---

# ★ w642 — MEASURED AFTER THE MULTI-GPU REWORK (2026-09-13)

Boot `w642a` on 50835305, rev `8be6e405`:

```
COUNTER-PAGE installed at gpa=0xfbbb0000 mmap_len=0x10000 slot=0x1000
BAR0-READ-HOTSPOTS  pages_touched=1  reads_from_live_pages=33  reads_from_BACKED_pages=0
                    top[+0xbb0000=33]
BAR1 (translated)   0 reads / 0 writes
BAR2 (translated)   0 reads / 0 writes
W392D_GUEST_OUTCOME=(P)        MEAN_FALSIFIER=PASS
```

★ **Goals 2 and 7 hold, and the per-GPU rework cost nothing.** The only BAR0 reads left are the
counter page, now served from the host's own register window with no exit.

## Goal 4 — the DoorbellTable: scoped, not yet wired

`crates/kayfabe-device/src/dbtable.rs` (316 lines, **zero production callers**) already has the
exact three arms the owner specified on 2026-09-13:

| owner's rule | `Route` arm | status |
|---|---|---|
| *"an invalid doorbell write registered in neither channel … ignore"* | `Unallocated` — bounds check, atomic load, return | ⊘ **built, not wired.** Today an unknown token still costs a queue slot and a worker refusal (`FwdFault::UnknownVchid`) |
| *"if the queue is full … set a flag … telling worker to ignore the doorbell queue and sweep all channels"* | `Emulated { chan }` | ★ **ALREADY LIVE** — `Offered::Full` ⇒ `DROPPED.arm_emulated_sweep()`, `shim.rs` |
| *"passthrough doorbells are inline in vcpu, no queue, no worker"* | `Passthrough { host_token }` | ⊘ **not wired.** `ring()` defers everything; `Worker::ring_doorbell` is an IPC round-trip |

### ⊘ Two findings that change how it must be wired

**1. The table is indexed by vChid, and a vChid is a PER-GPU namespace.** `route_doorbell` decodes
a raw token to `{vchid, runlist}` via pure arch math (cheap, lock-free, vCPU-safe), then looks up
`(GpuId, VChid)`. So the table is **one per device**, which is the same axis w641 just repaired
across `ViewSpace`, `SlotNumberSpace` and the KVM descriptor. Indexing by raw token would need an
array the guest's imagination sizes.

**2. Inline passthrough became possible only at w641.** It needs a writable VMM mapping of the
host's usermode doorbell page — which is exactly what `export_usermode_view(write: true)` +
`install_device_window` now provide. The owner said so first: *"I think you need same page, to do
doorbell mmio write in vcpu thread."*

### ⚠ The dangerous failure, and the discipline it demands

An **incomplete** table drops a doorbell for a channel that really is allocated — silently, and
only under the load that populated it late. ⇒ Wire it in **shadow first**: consult `route()` on
the vCPU, count its answer, keep today's path making the decision, and print an agreement census.
Flip the arms only once a boot shows the table and the worker agreeing on every doorbell.

★ This is the tree's own standing lesson applied before the fact rather than after: a green path
that has never been contradicted is not evidence, and `dbtable`'s tests pass today while the type
has never seen a real token.


---

# ★★★★★ GOAL 4 — THE TABLE IS WIRED AND MEASURED (w643a, w645a, 2026-09-13)

```
w643a  DBTABLE-SHADOW consulted=360 [unallocated=0 passthrough=204 emulated=156 malformed=0]
                      disagree=0  rebuilds=41746  rows=0
w645a  DBTABLE-SHADOW consulted=359 [unallocated=0 passthrough=204 emulated=155 malformed=0]
                      rows_peak=11  rebuilds=41713  skipped=41649
both   MEAN_FALSIFIER=PASS   W392D_GUEST_OUTCOME=(P)   BAR1 0/0   BAR2 0/0
```

## What the two boots establish

- **`unallocated=0` with `consulted` ≈ 360, twice.** The table would have dropped nothing over a
  boot whose live path served every doorbell. Both arms exercised (204 / 155), so the zero is not
  a table nobody asked.
- **The vCPU cost claim survives its falsifier.** `MEAN_FALSIFIER=PASS` and BAR1/BAR2 at zero with
  the consult on the doorbell trap path.
- `rows_peak=11`, `skipped=41649 / 41713` (99.85 %): the table really was populated, and the
  store pass now runs 64 times instead of 41 713.

## ⊘⊘⊘ WHAT THE ZERO DOES **NOT** ESTABLISH — and this is the load-bearing half

The rebuild ran **before** the birth drain in the same worker pass, so a channel born in pass N
only entered the table at pass N+1. **A doorbell arriving in that window decodes to a real vChid
the table calls `Unallocated`** — under the flip, a live submission dropped, which is goal 4's one
dangerous failure, created by the order of two statements.

⚠ `unallocated=0` over 359 doorbells **did not rule this out.** The window is narrow, so the zero
**bounds the race's rate and says nothing about whether it exists.** The defect was found by
reading the loop, not by reading the census.

★ **A green number is not a proof about a race.** Same family as *"a census zero needs a
known-positive"*, one step further out: here a known-positive would not have helped either,
because the event is timing-dependent rather than path-dependent.

⇒ Fixed by moving the rebuild **after** the birth drain (w646).

## Before any arm is flipped — the standing preconditions

1. `DBTABLE_SHADOW_*` are `static`s. With two GPUs the line sums both devices and `unallocated=0`
   stops distinguishing *"this device dropped none"* from *"the other device's traffic swamped
   it"*. They must become port fields, or the evidence for flipping is a number about two GPUs.
2. `Unallocated` must **not** mean "drop" on the first flip. A conservative flip short-circuits
   only `Passthrough` and lets `Unallocated` fall through to today's queue — which costs the same
   queue slot it costs today, and keeps any residual rebuild race harmless while the census proves
   it out.
3. Inline passthrough (the owner's *"no queue, no worker"*) needs a **checked** store accessor
   into the VMM's writable usermode-page mapping. The mapping exists as of w641; the accessor does
   not, and safe code may not touch a raw VMM pointer unchecked.
