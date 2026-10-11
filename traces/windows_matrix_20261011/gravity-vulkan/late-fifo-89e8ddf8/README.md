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

## Live smoke observation, 2026-10-11 21:51UTC

Gears/vectorAdd/CUP8 PASS with zero per-app153/WER/GSP cycles. Torch original
workload has no final verdict yet, but the no-capacity-refusal hypothesis is
already **FALSIFIED**: slot48 reports1049632 runs versus hard walk ceiling1048576,
then1049630; split9194 refused TRUNCATED/refuse_mask0x10, no commit applied.
This differs from the previous optional-growth-headroom exhaustion. At captured
prefix: no slot-pool-exhausted line, no GSP restart, unreconciled maximum1;
whole-boot invalidate max220424us is not fault-time causal proof. Do not claim
allocator fix makes the full app pass. Source WalkCfg default walk_pool=2^20
is itself this limit; simply removing per-space validation would exceed the
allocated walk/scratch pool. A capacity design must remain bounded and honor
§AD/§AE, not raise a threshold just to hide this symptom. Final app and host
fault/recovery facts remain pending. smoke-live-observation is a dated prefix,
not the stopped-run log. Method0x200 total includes unrelated Translated logs;
only candidate_methods_0x200 lists late RELAY-PEEK samples, currently none.

## Final smoke1490, 2026-10-11 21:52UTC

Original app verdicts: gears/vectorAdd/CUP8 **PASS**, torch_ai_bench **FAIL**,
411s, guest-rebooted, GSPcycle1, no successful final workload/readback proof.
Precise kernel journal window21:41:47–21:52:24.991301UTC query0, Xid0.
The harness's guest153/WER0 for the lost app session is not proof of clean
recovery; boot changed and one GSP cycle was observed. Bugcheck cause is
UNMEASURED; `bugcheck?` in the lane note is a question, not an identified stopcode.
Subsequent broker shutdown was clean, separate from that unexpected app reboot.
See smoke-result for unchanged app/oracle results, launch flags, public QEMU
log, selected capacity/refusal lines and source-prefix hash. New hard-ceiling
refusal precedes recovery; complete reboot causality remains unproven.
Gravity control1495 and probe1510 follow, independently of this failed smoke.

## Control1495 live observation, 2026-10-11 22:01UTC

Fresh original workload reproduces hostXid32/channel4e intr00800000 at21:55:37UTC
and one GSP recovery. Final600s oracle pending. Channel c1d0007a:ff040025 is
Passthrough/Windows-user-work, guest token27, initial32768-entry ring. Its first
5080 object ff1fe010 is host-authored/inert; FIFO-update API4000002d registration
returns0. Relay release later logs GET=PUT0x789e; this is not execution proof or
a successful app completion. Late sampler off in control; no late candidate
method recorded. Prefix counter maxima: birth_refused_cap0,unreconciled0,held0,
named_missed152; invalidate boot max115329us, no fault-time correlation.
Host journal query0/Xid1 in saved live window. See control-live-observation.
No capacity ceiling in this control prefix: distinguish it from smoke1490.

Initial Linux fast attempt has no usable suite output because output-directory
creation was missing. It is classified as setup failure/no executed-arm evidence;
no original NOTRUN tally is claimed. Corrected suite alone supplies30/30 PASS.
