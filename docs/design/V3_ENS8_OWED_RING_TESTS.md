# Translated-slot contention must preserve its consumed ring

**STATUS: RESEARCH, 2026-10-27.** GPU-free validated. ENS8 strengthens PR168 on top of its original head `899b0b5ba60884993c1e5a2970eea25ea311ea07`; no host run or PR retarget is performed here.

## Defect and exact witness

CODE READ, `ChanPlane::serve`, `owe_ring`, `HostIdx` and `Plane::worker_pass`: a worker consumes the bitmap hint and claims the Translated token before attempting the channel slot's mutex. Before PR168, a contended `try_lock` returned without preserving the ring. The worker then released BUSY to IDLE, with no bitmap bit, future interrupt or other pump source. A host fence may already be released; a channel pump is still needed to observe it. A guest doorbell has the same loss.

```text
slot held by another thread
host marker released / guest PUT advanced
one completion / doorbell edge: token RUNG, bit published
worker scan consumes the bit; claim makes BUSY
serve try_lock contends; old code returns without re-owing
release makes IDLE; pending work has no owner, queue, bit or later event
```

The EXPECTED RED commit `c980a5d45` executes the old drop adapter against the production token transitions. Both behavioral witnesses fail; the four original map/source-shape tests pass. In the later production-helper drop mutant, those four guards still pass while eleven behavioral tests fail, directly demonstrating the added coverage. The native PR168 implementation is preserved in that test-only checkpoint. The next change factors its existing recovery into the tiny `hostidx::reowe` callback shared by the native path and the runtime harness. The native path still resolves the guest token without acquiring the slot mutex, increments the same counter, rings once and returns false. No return value, route, timeout or scheduling policy changes.

CODE READ, `TokenWord::ring/release`: re-owing BUSY sets BUSY_RUNG. Release either acts again or republishes at the existing `REACT_ROUNDS` bound. When an unlocked pump consumes the pending work, the token becomes idle. Repeated rings coalesce; ringing twice is not itself a double pump.

## Model and production integration

The test is in `kf-qemu/src/hostidx_model.rs` so it can use the real `HostIdx` without adding a dependency from `kf-chan` back to `kf-qemu`. It follows the existing `kf-chan/tests/relay_fairness.rs` method: explicit actor microsteps, breadth-first exhaustive exploration with a visited set and shortest-prefix diagnostics, adversarial event/lock/teardown/stale-hint choices, plus seeded replay. It uses the actual `TokenWord` claim/ring/release implementation for every transition; it does not replace those operations with a hand-written token protocol.

The scheduler chooses scan, claim, serve, release, external edge, stale bitmap hint, slot-holder release and peer logical retirement. A worker's private queue includes entries carried while serving another token. One or two workers and up to two live channels are explored. A pending live obligation must have an owner, a worker queue entry or a published bit at every reachable state. Token ownership excludes a second pump. Logical retirement explicitly cancels only that channel's old obligations; it must preserve every live peer's debt. Retired and reused host identities are tested separately.

Liveness is conditional on a finite lock hold and continued worker scheduling. For every reachable adversarial prefix, the holder releases, remaining external edges are dispatched, and a fair round-robin suffix must pump pending work and quiesce within 128 actor microsteps. No new wake, timer or safety-net poll is added. This is not a promise of progress under an infinite lock hold or an indefinitely descheduled worker. Exhaustion of the one-million-state test ceiling fails as inconclusive; no unexplored frontier is counted green.

Two additional runtime tests run the actual `Plane::worker_pass` with its real token words and bitmap, while another thread owns a real slot mutex. They cover the completion path and the actual vCPU doorbell trap path, in both Vector and device RunlistVector modes. The latter maps raw doorbell0x10009 to guest index0x809, rather than treating the raw doorbell or the host token as the table index. The first visit returns after exactly `REACT_ROUNDS+1` contended calls, leaves a rung token, and never forges a Translated completion. After the holder releases, debt alone causes one pump/effect; a further pass does no work. The test releases its mutex before joining even on a broken/blocking mutant; its 2s test watchdog is not a product timeout or a correctness wake source.

Hostile-token tests ring a BUSY token 10000 times and issue 10000 unknown-host lookups: one token word and one live reverse mapping remain. The owed state uses the existing fixed token/bitmap tables. Native reverse-map insertion/removal follows channel birth/retirement and the declared channel cap; no guest doorbell or completion allocates a reverse-map entry. `get` returns a copied index before the callback runs. The recovery helper allocates nothing and acquires no lock. The original map lock is worker/act-side; the vCPU trap uses the existing atomic path and never reads it.

## Scope of mutation evidence

The six mutants exercise actual copied production source: drop recovery; wrong guest identity; release that perpetually republishes; peer map erased on slot teardown; claim that admits a second BUSY owner; and retained retired identity. Claim/release mutations target the production token layer inherited by the fix, not a policy flag in the model. The native worker-thread cases use the normal linked Plane; token-layer mutation cases use the copied token implementation in the explicit-state model. Every mutant must kill at least two tests; compiler failures are not killers.

SC actor exploration, real synchronization tests and these finite campaigns are falsifiers, not proofs of every weak-memory, GSP, driver, native free/refusal, or teardown-timer behavior. The model assumes an otherwise eligible, scheduled/enabled Translated channel. Stop/disable, general native teardown/refusal and guest-driver protocols are outside this contention proof. The model's completion marker is already released before its edge; it does not simulate GPU execution. Existing relay, handoff and lifecycle regression tests remain required.

## Hardware and branch interaction

MEASURED by TUR1, 2026-10-27, lease 57908255, `traces/tur1_20261027/README.md` on `claude/tur1-20261027`: the one fixed run lasting 17 minutes had contention but no overdue dump; Windows reset storms continued. ENS8 claims no new hardware result and no cure for all TDRs. PR169 (`6adec88f9a5f0b847084fbcc8f3d0365b49faa0a`) adds diagnostics only; it is read, not merged here.

CODE READ / merge dry-run, ENS8 CPU campaign 2026-10-27: PR168 auto-merges its code with ENS5 `6d95870c7`, ENS6 `21b17af213`, and ENS7 `d2ea6d26b`. Each has a content conflict only in `docs/STATUS_AND_HANDOFF.md`. The prospective code retains owed-ring recovery, early timeslice/mirror handling and end-refresh hooks; ENS7 also retains VRAM policy and current deficit fields. PR169's dry merge has no conflict. Exact outcomes are recorded in `traces/ens8_20261027/merge-analysis.json`. Dry merging is not compilation or runtime validation of the combined branches; the controller owns reconciliation and combined checks.

ENS5/6/7 can alter how long locks are held, so they can alter contention exposure. The recovery protocol remains the same: a live pending ring cannot disappear merely because a slot was momentarily unavailable. No memory-manager or mirror code is changed by ENS8.
