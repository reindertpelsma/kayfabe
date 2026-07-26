# QEMU BQL spike — what dispatching our MMIO without the big lock actually costs

**What this file is.** The measured detail behind `../design/l1_os_shell.md` §6.3's
foreign-lock contract, on the **QEMU** instance of that class. It answers three questions the
design had left as instructions rather than facts: does the mechanism §6.3 named exist on our
target, what does dispatching without the BQL actually buy, and what does it break. Measured
**2026-07-26** on the serialized vast.ai bench (KVM + GA106 / RTX 3060, host driver
580.159.04), against **QEMU 9.2.0** — the version in `/opt/qemu-src`, pinned by
`scripts/build_qemu.sh:9`.

**Why it is a reference and not a design doc.** §6.3 told the L2 adapter to *"verify it exists
and behaves as expected on the target QEMU version — this is an API-availability check, not an
assumption"*. The check was run and the answer was **no**. Where this file and a design doc
disagree, **this file wins and the design doc gets amended** — the same rule as
`mode2_bench_lifecycle.md`.

Tags as elsewhere: **[measured]** on hardware, **[src]** read from code, **[inferred]** a
conclusion drawn from those.

> **★ Read every absolute latency here as inflated ≈10×.** The bench is itself a KVM guest, so
> every vmexit pays the nesting tax the C already recorded (`mode2_baremetal_32`: the .32
> bare-metal run showed the *whole* observed gap was nesting). **The ratios are the signal;
> the microsecond values are not portable to bare metal.**

---

## 1. ★★ The mechanism the design named does not exist, and has not since 2020

**[src]** `memory_region_clear_global_locking()` was **removed in QEMU 5.2.0**, commit
`4174495408af` (2020), **as dead code**. Its only in-tree user had been the ACPI PM timer, and
that use was **reverted in 2016** after it caused real bugs. So the API the design cited as
"VFIO and virtio use it for exactly this case" had, by the time the design was written,
neither users nor an implementation.

**[src]** It is **absent from 9.2.0**. In 9.2.0 the dispatch path takes the BQL
**unconditionally**: `prepare_mmio_access()` (`system/physmem.c:2713`) acquires it with no
per-region opt-out. The KVM exit *itself* is already BQL-free —
`accel/kvm/kvm-all.c:3182` is explicitly commented *"Called outside BQL"* — so the lock is
re-taken **below** the exit, by the memory-dispatch layer, and nothing in 9.2 lets a region
decline it.

**[inferred] Pinning is not an option.** The last release that has the function is **5.1**
(Aug 2020). Shipping against a five-year-old QEMU to get one four-line behaviour is not a
trade anyone would take, and it would forfeit every other fix in 5.2→9.2.

## 2. ★ Upstream reintroduced it — in 10.2.0, under a new name, and with a second step

**[src]** Commit `73c520b08887` (Aug 2025, **QEMU 10.2.0**) adds
**`memory_region_enable_lockless_io()`**. It is the same capability with two properties that
matter to us:

1. **KVM-path only.** TCG ignores the flag. That is fine — we are a KVM device — but it means
   a TCG-based test of this behaviour proves nothing.
2. **★★ It also sets `disable_reentrancy_guard = true`.** Not incidentally. See §4: without
   that second step the first one silently drops guest memory accesses.

**[measured]** The **backport to 9.2.0 is ~4 lines and applies cleanly**, with
`system/physmem.c` the only load-bearing hunk.

## 3. [measured] Stock 9.2 holds the BQL in every handler; with the backport it holds none

A throwaway PCI device with three MMIO handlers, each asserting `bql_locked()`:

| build | `bql_locked()` observed in the handlers |
|---|---|
| stock 9.2.0 | **1** in all three |
| 9.2.0 + backport | **0** |

This is the direct observation, not an inference from latency.

## 4. ★★★ The silent-data-loss hazard — BQL-free dispatch and the reentrancy guard are a PACKAGE DEAL

The obvious hand-rolled alternative to the backport is to wrap the blocking call in
`bql_unlock()` / `bql_lock()`. **On latency it looks correct. It is not correct.**

**[src]** QEMU carries a **per-device re-entrancy guard** (`system/memory.c:551-561`) that
rejects an access arriving at a device that is already inside a dispatch. It is keyed on the
**device**, not the region — so **any two vCPUs touching any two regions of the same device
collide**.

**[measured]** With the manual unlock/lock and the guard left in place:

> **34 825 of 73 394 (47 %) of vCPU B's MMIO reads returned `MEMTX_ACCESS_ERROR` instead of
> dispatching**, each accompanied by `warning: Blocked re-entrant IO on MemoryRegion`.

A `MEMTX_ACCESS_ERROR` on a read is not a fault the guest sees as a fault: it is a value the
guest reads that we never produced. **Half the guest's reads of our device silently returned
garbage.** This is why upstream's `memory_region_enable_lockless_io()` sets
`disable_reentrancy_guard` in the same function.

> **The rule: never take one without the other.** Dispatching without the BQL and disabling
> the per-device reentrancy guard are one change, not two. Splitting them does not degrade
> performance — it **silently drops guest memory accesses**.

**★ The consequence, which is the real finding.** The reentrancy guard was **implicitly
serialising our device**. Remove it — as we must — and nothing outside our own code serialises
concurrent vCPU entries into the same device any more. So:

> **Once lockless IO is taken, `l1_concurrency.md`'s R1 (no blocking under a lock), R3 (lock
> rank) and R5 (revalidate after a lock gap) stop being latency disciplines and become
> CORRECTNESS requirements.** Before the change, a violation was a stall. After it, a
> violation is a data race with two real vCPUs in it.

## 5. [measured] The acceptance measurement

Shape: vCPU **A** blocks 5 ms at 50 % duty inside a handler; vCPU **B** hammers an
**unrelated** MMIO page of the same device for 5 s. B's distribution is the I-NOAMP canary
§7.9 prescribes.

| Arm | B p50 / p99 | B throughput (5 s) | Note |
|---|---|---|---|
| stock 9.2.0 | 142 µs / **5 611 µs** | 13 876 ops | p99 ≈ **exactly A's 5 ms block** — B is queued behind A through the BQL. **5.3× degraded** |
| + backport | **60 µs / 146 µs** | 72 633 ops | indistinguishable from the A-idle control; A's 5 ms **never appears in B's tail** (max 758 µs) |

**★ What makes this conclusive rather than suggestive** is not the ratio, it is the *shape*:
the stock p99 is not "high", it is **A's block duration, transferred onto B**. The mechanism is
visible in the number. With the backport that number is gone from the distribution entirely,
not merely reduced.

## 6. ★ ioeventfd for the doorbell — and the token question, resolved favourably

**[src]** `memory_region_add_eventfd()` **exists in 9.2**, works on a sub-range of a PCI BAR
MMIO region, and with `match_data = false` matches on **address + size alone**. It carries
**no payload**: the handler learns that a write happened, never what was written.

That normally disqualifies a doorbell whose written value is a channel token. **Here it does
not, because the C already ignores the value:**

- **[src]** `nvkvm_m2_exec_doorbell(s)` (`C: src/qemu/nvkvm_gpu_emul.c:8644`) takes **neither
  offset nor value**. The written value feeds **only** an off-by-default debug log.
- **[src]** The C already re-enters that path with a **fabricated `0`** (`:3366-3369`) — i.e.
  the value-free entry is not a new mode, it is the existing one.
- **[src]** It re-derives *which* channel advanced by polling **every** channel's `GP_PUT`
  (`:8777`) against its shadow `gp_get` (`:8863`).

**Register without datamatch.** Tokens are per-channel and churn, so a datamatch registration
would need one eventfd per live token and re-registration on every change — all to recover
information the handler discards.

**[measured]**

| | trapped | ioeventfd |
|---|---|---|
| doorbell p50 | 50 µs | **29 µs** |
| throughput (5 s) | 79 306 | **143 556** |
| under A's 5 ms BQL block | — | **28 µs p50 / 86 µs p99 — unaffected** |

**[inferred] Coalescing is safe here.** ~0.1 % of writes coalesce (two writes, one wake). The
handler is **level-triggered over `[gp_get, GP_PUT)` and idempotent** — it processes whatever
the shadow says is outstanding — so a merged wake loses nothing. *This is a property of this
handler, not of ioeventfd.* An edge-triggered or value-consuming handler behind an ioeventfd
would be a bug.

## 7. ★★ The caveat that must travel with §6: ioeventfd frees the vCPU, not the SERVICE

**[measured]** The ioeventfd handler runs on the **main loop with the BQL held**
(`bql_locked()` = 1). ioeventfd removes the *vCPU* from the critical path; it does **not**
make the work BQL-free.

> So ioeventfd **relocates** the stall from the vCPU to the main loop unless the service
> behind it also (a) never blocks under the BQL, or (b) runs in an IOThread. On its own it
> converts "one vCPU is stuck" into "the main loop is stuck" — which is worse for everything
> else in the VM, not better.

The measured "unaffected by A's block" row in §6 is *B's doorbell latency*, and it is real; it
is **not** evidence that a blocking service behind that doorbell would be harmless.

## 8. ★ NAMED UNKNOWN — data-carrying BAR1 writes were not measured

**[src]** BAR1 aperture writes — including the guest's own `GP_PUT` store
(`C: src/qemu/nvkvm_gpu_emul.c:4407`) — are **data-carrying**: the written value is the
payload. They therefore **cannot** use ioeventfd and stay on the vCPU thread, under the BQL,
in stock 9.2.

**[inferred]** Their handlers are fast (a GMMU walk), so they are *probably* fine. **That was
not measured**, and "probably fine" is exactly the shape of claim this project's docs are
supposed to refuse.

> **The experiment that settles it:** the §5 A/B harness, with B's handler doing
> **page-walk-sized** work instead of a trivial read, driven at **realistic BAR1 write rates**
> taken from a real Mode-2 workload. Report B's p99 with A blocking, both arms.

## 9. What this spike does NOT establish

Stated so nothing here is over-read:

1. **It is one synthetic device, not our device.** The handlers were throwaway; no GMMU walk,
   no isolate round trip, no real guest driver on the other end.
2. **The ioeventfd numbers are a microbenchmark of doorbell writes**, not of a workload. They
   establish that the *mechanism* works on a PCI BAR in 9.2 and that the token is recoverable;
   they do not establish an end-to-end speedup.
3. **Nothing here was run on bare metal.** See the nesting caveat at the top.
4. **The backport was measured for behaviour, not soaked.** "Applies cleanly and the flag
   works" is not "carried across a QEMU upgrade for a year".
5. **No claim about TCG.** The upstream flag is KVM-only by construction.
