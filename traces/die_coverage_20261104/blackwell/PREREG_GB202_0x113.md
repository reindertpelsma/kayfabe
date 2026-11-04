# GB202 Windows bugcheck 0x113: repeat-run pre-registration, 2026-11-04

STATUS: LIVE, 2026-11-04. Written at 02:23Z, before the lease below ran anything. Follow-up to Finding 1 of
`README.md` (GB202 gate r1 at `0c515b3b`: bugcheck 0x113 about 20 s after the enable; r2, r3 passed).
Revision `0c515b3b3` (pin branch `claude/die-coverage-blackwell-pin-20261104`), no code change.

⊘ CORRECTED 02:33Z (before any run): 59044337 and the next try 59045484 (offer 59030076, Spain) were created STOPPED
(`success False`; `vastai start` answered "Required resources are currently unavailable"); both destroyed by id, never
ran. The lease is **Vast 59045517** (own id), offer 49824506 (RTX 5090 `10de:2b85`, Czechia, machine 35231, 1.004 USD/h
listed, 15 vCPU, 49 GB), created 02:31:27Z, safety timer 7200 s; also a different machine from 59031351.

Lease: Vast 59044337 (own id), offer 58921244 (RTX 5090, Australia, 0.739 USD/h listed), created 02:21:53Z, a
DIFFERENT machine from the first GB202 lease (59031351, machine 151715). Safety destroy timer 7200 s.

Procedure: `windows_lane.sh` (provision + one install), then `windows_gate.sh` by hand, back to back, each run on a
fresh overlay of the same installed base (the harness's default), until a second bugcheck, the deadline (report by
05:30 CEST = 03:30Z), or 5 runs. **Deviation, declared now:** `KF_WINGATE_IDLE_S=60` instead of 300, to fit more runs
before the deadline. The r1 crash happened in the enable stage (before the idle), so the shortened idle does not hide
that failure mode, but these runs are NOT full gate passes and are reported as "gate runs, idle 60 s".

Per run recorded: verdict; whether a WATCHDOG dump appeared (`GATE_WER_DUMP`); crash time relative to the enable;
the kf3 log tail before it; host Xid delta; WER / LiveKernelEvent codes and the gate's reset counters (`GATE_TDR`).

Hypotheses and falsifiers:
- **H-rate:** GB202's failure rate is materially above GB205/GB206 (0 of 4). With n this small only a strong signal
  counts: **supported** if >= 2 of the new runs fail (then >= 3 of 8 on GB202 against 0 of 4); **falsified as
  "material"** if 0 of >= 4 new runs fail (1 of >= 7 on GB202, indistinguishable from 0 of 4); 1 failure = undecided.
- **H-precursor:** passing GB202 runs show a precursor of the crash. Falsifier: no passing run has a WATCHDOG dump, a
  LiveKernelEvent other than 193, or a kf3 `display life ENDED` / RM unload line between the enable and the gate's end.
- **H-timing:** the failure follows the enable by under 60 s (driver start / first modeset), not a later stage.
  Falsifier: a bugcheck or reset at a later stage (probes, idle, post-idle).
