# Walk wake after #239 — preregistration (written 2026-11-03, before any code change or run)

STATUS: RESEARCH, 2026-11-03. Hypotheses and falsifiers for the walk-completion latency fix. Results are folded
into `README.md` in this directory; nothing here is evidence.

## Why the callback went (CODE READ of history, not a hypothesis)

`fc5c83379` (import of `ffd72e3d4`, sweep-20261018) removed the per-walk `cuLaunchHostFunc` eventfd signal and
made `cuEventQuery` on a freshly recorded event the only proof of completion (`completion::ready` ignores the
fd). The reason is the TU106 R34 experiment (`7ecc53efd`, result `e16879a80`,
`traces/die_driver_sweep_20261015/turing_hold_20261016/R34_RESULT.md`): the host callback raised one GPU
nonstall interrupt per walk (raised IRQ 45,190 vs 3 without it), which the guest is shown and acknowledges by
trapping register writes. Two properties must be kept:

- **P1 (no forged/stale completion):** only the current walk's GPU event proves completion; a wake source is a
  hint, never proof; a query error refuses the walk and poisons the walker.
- **P2 (no per-walk host interrupt):** no host function / blocking-sync wait per walk (that is what produced the
  NSI burst). Every real host nonstall edge is still delivered (nothing filtered).

The same R34 result ranked as the next experiment: *"a bounded shorter pending-event wait on the VA worker"*.
That is the fix tried here.

## The fix (stated before measuring)

While a walk is in flight, the VA worker (`kf3-vamgr`, never a vCPU, no lock a vCPU takes) waits in
`epoll_pwait2` with a **microsecond** timeout that grows with the walk's age: `clamp(age / 2, 50 us, 1000 us)`
(checks at about 50, 100, 150, 225, 340, 510, 760 us, ... then every 1 ms). The thread's timer slack is set to
a few microseconds so the kernel does not add its default 50 us. Idle (no walk) keeps the 50 ms backstop. No
spin, no host callback, `ready()` unchanged (P1, P2 kept). Kernels without `epoll_pwait2` round up to 1 ms (the
current behaviour), said once in the log.

## H1 — root cause: the 1 ms pending poll is the added walk latency

INFERRED today (Ada lane, `origin/claude/ada-pc-regression-20261103`): no wake is left for a finished walk, so
it is collected only when the 1 ms timeout fires.
**Prediction (Ada PC, RTX 4070, fast suite 30 arms, 180 s):** `vat[walk_avg_us]` <= 250 us on every arm with
walks; `gpu_avg_us` unchanged (81-199 us); suite client time <= 77,250 ms (pre-merge 67,176 ms + 15%);
`concurrency` invalidate arrival-to-clear <= 300 us.
**Falsifier:** with the log line confirming the microsecond wait is in effect, `walk_avg_us` stays above 500 us
on the arms with walks, or the client total stays above 100,000 ms. Then the cause is elsewhere and is reported
as such.

## H2 — no regression

- GPU-free: `cargo test` for `kf-linux-raw`, `kf-cuda`, `kf-qemu`, `kf-harness` pass. Falsifier: any failure.
- Fast suite 30/30 and the same 5 admitted host Xids as the 70e3fabd/9297ffdd runs. Falsifier: any arm not PASS
  or a new Xid shape.
- Windows (owner image chain, Ada-lane recipe): Code 0, probes pass, reset counters 0 (stale 78 LKE reports
  excepted as in the Ada lane). Falsifier: Code != 0, a probe failure, or a new reset.
- `kf3-vamgr` CPU: not more than +1 percentage point of one core over the Ada lane's numbers (Windows idle
  0.45 %, gears 1.09 %). Falsifier: more. INFERRED expectation: idle unchanged (no walk, 50 ms backstop), busy
  +0.1-0.5 points (2-4 extra wakes of a few us per walk).

At most 3 negative runs, then stop and report.
