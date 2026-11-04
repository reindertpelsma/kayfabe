# Flake hunt: `--concurrent-fuzz` in the thin fast guest

**STATUS: LIVE, 2026-11-04.** The pre-registration, written before the first run. The results go
in `RESULTS.md` next to this file.

## The observation (measured, by the coordinator)

Worker CI run 41763934006, rev 942ca5431, RTX 3060 (GA106), Quebec machine 22775, 2026-11-04: the
thin-guest fast suite printed `FAST_CELL_ARM arm=concurrent-fuzz verdict=FAIL secs=29
client_ms=8620` with `raw client rc=1`. Every other arm passed, and bare metal (30/30) and the
gates (9/9) passed on the same box. The same arm passed on the same machine in run 41749680326 and
in the other integration-revision runs that day. The job log holds only the trimmed arm output.

## Facts read before the run (measured from the source and from git, not from a box)

- **rc=1 is the graded exit, not the watchdog.** `concurrent_fuzz()` returns `verdict == "PASS"`
  and `main` maps false to `ExitCode::from(1)`
  (`crates/kayfabe-rm-ladder/src/main.rs`, the `want_concurrent_fuzz` block). The watchdog exits
  with code 3 through `process::exit(3)`, and a panic exits with code 101. So rc=1 is one of these:
  - `FUZZ_REASON=INVARIANT_VIOLATED`: an arm FAILed, with a named violation or
    `WORKER_DID_NOT_FINISH`.
  - `FUZZ_REASON=CONTROL_FAILED`: the single-thread control failed, and the verdict is NOTRUN.
  - `FUZZ_REASON=ARM_UNMEASURED`: some arm was graded NOTRUN with `NO_CONCURRENCY_OBSERVED`,
    `NO_ENGINE_WORK_SAMPLED` or `PIN_REFUSED`.
  - An early refusal: `NO_ENGINE`, `NO_DEVICE`, `NO_SECOND_CLIENT` or `NO_VASPACE`.

  **NOTRUN exits 1 just as FAIL does**, so the fast suite scores an unmeasured arm `FAIL`.
- **client_ms=8620 lies inside the range of passing runs** in `traces/` (1279–8160 ms on
  various boxes). Inferred: the arm ran to completion, and the client was not wedged.
- **The failing rev does not contain PR #241 (walk wake-up, adaptive sleep).** 942ca5431 is not a
  descendant of `4f420b373` ("fix(walk): look at a pending walk on its own time scale"), of
  `1d862797f` (the #241 merge) or of `a12dddd24` (#248). Its product code is pre-#241 `next`
  (`b23fa061e`) plus the display, caps and precomp merges (`git diff --stat b23fa061e 942ca5431`).
  Measured from git. ⇒ **The walk-wake change cannot have caused the observed failure.** "The
  integration tree plus the Windows harness" did not hold for `crates/` in that run.

## Hypotheses, each with its falsifier

- **H1. Instrument defect (an arm went unmeasured).** The failure is `FUZZ_REASON=ARM_UNMEASURED`:
  one arm sampled no cross-thread overlap or no engine work on a nested box, and the suite scores
  NOTRUN as FAIL.
  *Falsifier:* a failing iteration whose reason is `INVARIANT_VIOLATED`, `CONTROL_FAILED` or
  `WORKER_DID_NOT_FINISH`.
- **H2. Kayfabe defect (a real violation).** The failure is `INVARIANT_VIOLATED` or
  `WORKER_DID_NOT_FINISH`, with a named invariant (`VA_DOUBLE_BOUND`, `MAP_NOT_ATOMIC`,
  `ALIAS_*`, `CROSS_CLIENT_LEAK`, `FREED_HANDLE_RESOLVES`, `VA_NOT_RECOVERED` or `STALE_READ`) or
  an RM verb error.
  *Falsifier:* every failure is `ARM_UNMEASURED` or an early refusal.
- **H3. A host fault reached the arm.** A host Xid, for example Xid 31 from another arm's
  intentional-fault control, landed in the fuzz window.
  *Falsifier:* zero host Xid delta in the fuzz window of the failing iterations. One boot per arm
  means no other arm runs in that window by construction. The loop records the Xid delta of the
  pre-arm window and of the fuzz window separately.
- **H4. #241 is involved.** Already falsified for the observed failure (see the facts above). If
  the loop on `next` (which contains #241) shows failures, a loop on 942ca5431 or `b23fa061e` would
  tell whether #241 changes the rate. That loop runs only if time allows.

## Protocol

- One box: a GA106 (RTX 3060), preferably the same Quebec machine 22775. Provisioned with
  `provision_full.sh` on `KAYFABE_BRANCH=next` (the `kf3` binary under `kf3-bins/<rev>/`, the raw
  client and the fast guest built from that checkout).
- **Phase A:** 40 × `fast_suite.sh <tag> 180 --concurrent-fuzz`, one fresh QEMU each, back to back
  (`cfuzz_loop.sh A 40`).
- **Phase B (if A has 0 failures, else as time allows):** 40 × (`--map-stress` boot, then a
  `--concurrent-fuzz` boot): the suite's order, with the same boot flags (`cfuzz_loop.sh B 40
  --map-stress`).
- **Recorded per iteration:** verdict, secs, client_ms, client rc, FUZZ_REASON, seed, the per-arm
  `FUZZ_ARM` verdict and why, the host Xid delta per window, and the Xid codes. On a non-PASS:
  the full serial, the QEMU/kf3 log, ttyS0 and the dmesg tail.
- **The bound with zero failures:** the rule of three. 0 of n ⇒ p < 3/n at 95 %. That is 7.5 %
  for n=40 and 3.75 % for n=80.
- No production code changes. A root-cause patch, if one is evident, goes in a separate commit
  marked UNTESTED.
