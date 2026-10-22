# RC-03 / RC-02 / PR-2 service fairness

STATUS: RESEARCH, 2026-10-22. STOPPED after the prescribed shared-lock wait expired.

MEASURED stop: the final trap red check waited600s for the owner's global Cargo
lock and exited1 without starting Cargo. The own red-proof guard refused that
result. No retry, alternate lock or product implementation followed the refusal.
The two added prefix-conservation candidates are backed up but UNEXECUTED.

MEASURED completed focused baselines (separate valid per-crate commands):
core1 passed/3 failed; trap1 passed/7 failed; dbfast1 passed/2 failed.
All12 failures are assertions on unchanged master product code;3 controls passed.
The current branch has17 focused cases, of which the last2 remain unexecuted.
There is NO product fix, green acceptance, mutation campaign or final lint delta.
The proposed-token-scan.txt file is an UNVALIDATED source proposal only; it is
not compiled or installed by this branch. It was authored before the lock refusal.


INFERRED hypotheses, registered before runs: rotation must reach tokens behind
hot low tokens; a full bounded scan must leave an announced rescan; one ready
batch must return to drainer work. FALSIFIERS: starvation, lost/duplicate bits,
exceeding the scan limit, parking beside live inventory, a registered peer not
notified, permanent spin after stale hints drain, or refill monopolizing a call.

MEASURED: b5a068eb6 imports all three race-audit files unchanged with git checkout.
The core and trap audit tests fail by assertion on unchanged master product code.
The original PR-2 watchdog PASSED the first baseline after2.43s: a scheduled
producer can stop sustaining full batches long enough for the old loop to escape.
It is not a reliable red oracle. Two new tests re-signal all fds synchronously
inside delivery, seeded before service; both fail by assertion on master. They
also require readiness to survive to a subsequent turn. No real KVM is used.

MEASURED: e413cc026 adds peer/same-word/idle controls, within-summary victims,
wrap conservation and deterministic refill:8 assertion failures/3 passed controls.
f99147c46 adds two dispersed-crowd and two zero-budget cases; the completed
trap run reports7 assertion failures/1 passing control. Prefix-preservation
regressions were then added; their execution never started because the shared-lock wait expired.

CODE READ: word-only rotation is insufficient: attacker0..9 plus64..117 fills
64 slots, and resuming at word2 can wrap back to those same prefixes forever,
starving118/127. A token cursor splits the cyclic range at the last returned
bit. Excluded prefix bits and remaining suffix bits must both be re-published
word-before-summary; summary is still cleared before the word is taken.

CODE READ proposed protocol: a full positive-budget scan bumps the existing
wake word; SignalOne calls a required HostOps notifier method. Product QEMU and
the native harness use their existing nonblocking worker fd. Stale hints do not
inflate served counts. False-positive exhaustion costs an extra empty pass;
the stale-only positive control requires eventual park. No new timer or flag.

CODE READ proposed drainer protocol: retain command synchronization first,
perform one nonblocking readiness snapshot, deliver it, return. The existing
level-triggered eventfds stay ready for the next drainer iteration; no input
poll interval, timeout, readiness-array size or GP_PUT poll is introduced.

MEASURED: Clippy baseline completed on trap/core/chan/qemu/harness all targets:
305 warning fingerprints, including4 style warnings in imported audit tests.
Product baseline is unchanged. Remove those import warnings without changing
assertions; final delta, full-suite counts and mutant kills remain pending.

INFERRED hardware obligations later: production two-worker notification load,
channel churn, sustained optional ioeventfd doorbells with privileged input,
Linux/Windows workload/boot/reset acceptance. No hardware result is claimed.


CODE READ resume boundary: keep VA-1 and VA-4 DEFERRED as the owner requested.
After a later authorized resume, run the two pending prefix checks against master
product first; retain the EXPECTED RED history. Implement the token cursor, the
required nonblocking HostOps notifier seam in QEMU/native harness and model hosts,
and one dbfast readiness snapshot. Preserve audit assertions; remove the imported
style warnings. Then run the GPU-free suites with the existing KAYFABE_NO_KVM=1
capability gate, apply hand mutants with >=2 killers each, restore and recheck,
and compare Clippy source fingerprints with added0. Continue AT-1, SA-04/06/14,
SA-07, SA-01 last. No such remaining step is claimed complete here.
