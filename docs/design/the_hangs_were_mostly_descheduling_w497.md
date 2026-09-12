# w497 — the "23 ms MMIO hang" is mostly a DESCHEDULED vCPU. Measured.

**STATUS: LIVE, 2026-09-12.** Box `50685423`, RTX 3090 (GA102), **24 cores**, client `(P)`.

## The measurement

`TRAP-CPU` records wall and CPU time **for the same trap** — not two independent maxima,
which cannot be compared.

```
TRAP-CPU n=89220 worst_wall=16818us cpu_of_that_trap=656us slow_starved=47 slow_busy=18
```

★ **The worst trap spent 656 us on a CPU and 16 818 us in total.** The thread was not running
for **96 %** of it. Of 65 slow traps, **47 were starved** (CPU under a tenth of wall) and 18
genuinely burned CPU.

**Second, independent confirmation in the same run** — same revision, 24 cores instead of 11:

| | w472 box (11 cores) | w495 box (24 cores) |
|---|---|---|
| `slow_traps(>1ms)` | ~197 | **70** |
| `worst_trap` | ~23 ms | **16.8 ms** |

⇒ the number moves with **core count**, which no property of our handler can explain.

## What this retires

⊘⊘⊘ **Seven hypotheses failed because they were all hunting a 20 ms code path that does not
exist.** The lock throttle, the mirror walk, PRAMIN, the inline verbs, memslot churn, the
plane lock, the Device lock — every one was a search for work. Most of the interval was not
work.

★ And it explains the fact that survived all seven: `bar0+0x110094` has **no decode arm
anywhere** and still measured milliseconds. Code that does not exist cannot be slow — but a
thread that is not scheduled can still be measured slow, because `TrapGuard` measures **wall
time**, and it is installed inside our own entry point, so the interval it reports is
attributed entirely to our handler.

⚠ **This does not mean the traps were fine.** It means the number was not what it claimed.
Two separate problems were wearing one figure:

1. **Measurement/scheduling** — 72 % of slow traps. 8 vCPU threads plus a coordinator plus
   isolate processes against 11 cores. Not fixable in the handler, and not a guest-visible
   hang in the way a real 23 ms stall would be — though a descheduled vCPU *does* stall its
   guest, so it is not free either.
2. **Real work — up to 656 us of CPU in one trap, 18 traps over 1 ms of CPU.** ★ *This* is
   the remaining target, and it is sub-millisecond-adjacent rather than 23 ms.

## What to do next

- **Chase `slow_busy`, not `worst_trap`.** The honest budget metric is **CPU time per trap**.
  `worst_trap` conflates our cost with the host scheduler's.
- **Report both.** A wall figure alone will mislead the next reader exactly as it misled seven
  hypotheses.
- **Thread count is now a product concern**, not a bench artefact: the design spawns a
  coordinator plus isolates, and if those exceed the cores the operator gave the VM, guest
  vCPUs lose time. That belongs in the concurrency-plane work, not in the trap handler.

⊘ The 2 ms alarm pass produced no core (`core_pattern` is likely refused in this container).
Not needed — pass 1 answered it — but the alarm is still the right tool for the 18 traps that
really do burn CPU.
