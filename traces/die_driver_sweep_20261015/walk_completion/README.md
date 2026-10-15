# Walk callback experiment for the TU106 R34 timeout

**STATUS: RESEARCH, 2026-10-15 19:23UTC; CPU candidate passes, hardware unrun.**

Measured CPU: before3970f7859 compiles,5tests with3expected assertion failures
(stale signal, hidden CUDAerror,50ms pending sleep). Candidateffd72e3d4 passes
all5; full isolated kf-* suite3252pass/0fail/14ignored,207resultgroups, exit0
at19:20:58UTC. Trusted host2buildjobs, no GPU work. Logs before.log,after.log,
all-tests.log. Existing warning/formatting debt remains; noCI-green/mastermerge
claim. HardwareR34/Windows/Linuxbroker replay remainsNOTRUN.


Parent product8e3a9d325, evidence parent728e6febd. Own branch
codex/fix-walk-completion, worktrees/workspace/codex/walk-completion and
/data/codex/walk-completion, isolated target/data/codex/target-walk-completion,
trusted CPU host two build jobs. Owner GPU/VM untouched.

H-W1 stated before implementation: one CUDA host callback per walk causes the
observed GPU-wide GR0 nonstall wakes and guest IRQ acknowledgements that dominate
TU106's original-depth R34. Native raw R34 passed in4seconds; guest180second
budget times out, diagnostic300second budget passes in201seconds. Frozen grader
and13000decoys are unchanged. Long diagnostic counts51854walks,51866GR0wakes,
672876IRQ-register writes. These counts establish correlation, not causation.
Falsifier: removing the repeated helper callbacks does not lower wakes/traps or
original-depth time on the same rental. Compare full phases and retain failure.

Candidate: record a fresh completion event explicitly after each graph launch
on the same stream; no callback in the replay graph. Always query the real event
before reading the pinned report, propagate CUDA errors and preserve one walk
in flight, map/ack/invalidate ordering, GPU execution and every observed NSI.
VA worker and harness request at most1ms sleep while work is pending; idle
retains50ms backstop. This is a scheduling budget, not a latency guarantee.
No interrupt filtering or guest test-budget change. A one-time bring-up callback
can remain to warm/drain the existing completion fd; it is not a report proof.

Explicit post-launch cuEventRecord avoids relying on when a reused captured
event becomes pending. NVIDIA documents that cuEventRecord replaces prior
captured work, and cuEventQuery reports success only when that work is complete:
https://docs.nvidia.com/cuda/archive/12.8.1/cuda-driver-api/group__CUDA__EVENT.html.
No closed binary investigation. The completion fd policy is extracted without
changing behavior first; tests demand event proof even for a stale signal and
1ms checks while pending. Those assertions must fail before the change.

Acceptance remains native reference, gates9/9+gate10, exact revision raw30/30
at180seconds, CUDA4 and broker/Windows regressions. No performance or family
acceptance claimed from a model or from a longer diagnostic budget.
