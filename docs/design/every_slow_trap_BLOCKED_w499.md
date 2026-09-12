# w499 — every slow trap **blocked**. Not descheduled, not stolen. Measured.

**STATUS: LIVE, 2026-09-12.** 24-core box, ~90 % idle, client `(P)`.

```
TRAP-CPU n=89157 worst_wall=14796us cpu_of_that_trap=908us
         slow_starved=54 slow_busy=18 slow_preempted=0 slow_blocked=72
```

★★★ **72 of 72 slow traps carried a VOLUNTARY context switch. ZERO carried an involuntary
one.** The thread **gave up the CPU** — it waited on something. That is a futex, which means
a lock, in our code.

## What this refutes, including my own conclusion one hour earlier

⊘⊘ **w497's "the hangs are mostly descheduling" was WRONG.** The wall-versus-CPU gap it rested
on (656 us of CPU inside a 16 818 us trap) is equally consistent with **blocking**, and I read
it as the answer instead of as two candidates. `ru_nivcsw` separates them and says
**preemption never happened**.

The owner's objection is what forced the check: *"context switching time is in sub
milliseconds, not 26ms right"* — correct, and the right conclusion from it was not "so it must
be something else exotic" but "so measure which kind of not-running this is".

Also ruled out along the way, each measured rather than argued:

| layer | measurement | verdict |
|---|---|---|
| cgroup CPU quota | `nr_throttled 0`, `throttled_usec 0` | not throttled |
| hypervisor steal (the box is itself a KVM guest) | `STEAL-TAIL worst_single_event=10.0ms`, total 1320 ms / 150 s | **real, and it is the WALL-CLOCK NOISE FLOOR** — but steal does not cause a voluntary switch |
| scheduler preemption | `slow_preempted=0` | never happened |

⚠ On steal, the owner's other correction stands and is methodological: *"0.03% says nothing,
only long tail matters."* An average cannot see a tail. ⇒ **wall-clock trap figures on a
rented VM have a noise floor around 10 ms**, which is where every "23 ms hang" chased tonight
was sitting. **CPU time per trap is the only honest budget metric**, and by it the worst trap
is **908 us**, not 15 ms.

## Where this leaves it

★ **The owner has been right since the first message on this thread:** *"23 ms mmio hang is a
hang, caused by our code, not being a simple queue + wake … locks taking over data structures
that protect 90% of the data completely irrelevant for the vcpu mmio handler."*

⊘ And the reason eight hypotheses missed it: **`lockcost` instruments only RANKED locks.** An
unranked `std::sync::Mutex` records no wait, no hold and no site, so rank 0 read as innocent
every single time. The blocking lock is invisible to every instrument built tonight.

**Next, and it needs no guessing:** the per-trap alarm (`KAYFABE_STALL_ALARM_US`) fires while
the thread is still parked in the futex, and `SIGALRM`'s default action dumps **at the
blocking call**. That names the lock and the line. ⊘ Do not guess which lock — the last eight
guesses were all wrong and one of them was announced as found.
