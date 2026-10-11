# Exact89e8ddf8 regression and matched late FIFO probe

STATUS: RESEARCH, 2026-10-11. Campaign in progress; no new Windows verdict yet.

Source89e8ddf8b55d0b501d836c95ce40b3114eecab8e includes actual-demand allocator
and the default-off, independently reviewed late measurement diagnostic.
Trusted172.22.1.20 RTX4070/host595.91.07; Windows580.88. Installed binaries are
per revision, never the shared build directory. No compilation during app timing.
Shared GPU lock covers all serial runs; owner released manual1380. Personal
image chain preserved separately, never used for this campaign.

## Run questions stated before execution

1. Linux paths stay correct: falsifier is a failed actually started gate or fast
   arm on exact89. Gates9/9+gate10 passed21:34UTC; fast30/30 passed21:41:47UTC, zero FAIL/CRASH/NOTRUN. Initial runner omitted output-directory
   creation; fast NOTRUN/exit1 was preserved, not a product failure or pass.
   Corrected resume script starts fast30 after checking existing gate verdicts.
2. Smoke1490: exact revision boots gears/vectorAdd/CUP8 and the original torch
   workload without allocation refusal, recovery or host Xid. Falsifier: failed
   original readback/proof, capacity refusal, recovery or host Xid.
3. Control1495: original GravityMark Vulkan failure remains on fresh production
   baseline. Falsifier: original600s workload clean PASS with image/timing/GPU proof.
4. Probe1510: a late submitted packet on the faulting twin contains0x200 with its
   registered FIFO-update handle. A positively identified different faulting
   method contradicts this; absent/bounded/racy samples remain INCONCLUSIVE.
   Only KF3_RELAY_PB_PEEK=after-fifo-update differs from control. Guest-byte
   observations do not independently prove GPU fetch/execution or falsify H1.

Scripts are archived in ../h1-channel-audit/{run_campaign,resume_campaign,
start_after_manual,post_guest}.sh. Original600s app/oracle, media, tool overrides,
MPO disabled, and19 production behavior flags are shared by matched runs.
Default diagnostic remains off. No FIFO migration/native registration added.

Result artifacts will be folded here as measured; no whole-matrix or arbitrary
branch-tip green claim follows from these selected exact-revision runs.
