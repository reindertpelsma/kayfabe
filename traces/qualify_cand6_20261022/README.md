# Q1 candidate-6 qualification

STATUS: SAFETY STOP; QUALIFICATION INCOMPLETE, 2026-10-22.

MEASURED: candidate f7cf3ed81200077e55ee7769c635887661f3f990 fetched from origin. Q1 evidence branch: codex/qualify-cand6-20261022. Start 05:01 UTC; hard host-restoration deadline 10:00 UTC. No product edits authorized by this qualification.

INFERRED H1: the candidate is the stated three-fix integration. FALSIFIER (before checking): different remote head, absent fix ancestry, or failed required CI job. MEASURED: remote head agrees; CI run 39725599154 was queued at first check. CI and ancestry result pending.

INFERRED H2: all candidate GPU-free suites remain green and clippy adds zero diagnostics versus e64b8e5f6. FALSIFIER (before execution): any failed test, rustfmt failure, or added source-bound clippy diagnostic. Result pending.

INFERRED H3: merged regression tests discriminate each fix. FALSIFIER (before mutation): restoring the old descriptor budget, handle privilege authorization, or >64-space refusal kills fewer than two tests. Result pending.

INFERRED H4: candidate passes native 9+10 and Linux 30/30 under the approved exact-match intentional-fault policy. FALSIFIER (before execution): failed/missing verdict, non-USER birth, missing completion, or any new unmatched host Xid. Result pending.

INFERRED H5: all nine SPECviewperf workloads and available production regression smokes complete on the candidate. FALSIFIER (before execution): benchmark failure/kill, guest nvlddmkm 153 or Display 4101, new unmatched host Xid, or descriptor/scan budget refusal. Require exercised large-Pointer counters. Result pending.

CODE READ: lane requires native-before-guest, candidate-built binary, independent Windows CoW, production flags, pinned K1 runner, and no master/candidate push. Raw logs stay outside git. Measurements and limitations will be appended here.

CODE READ — hardware stop, before any GPU workload or VM boot: candidate `scripts/fastguest/run_fast_guest.sh:79` executes `pkill -9 -f '[k]f-fastguest'`. A process name supplies no ownership proof. This conflicts with the lane's own-PIDs-only instruction. The sweep branch's launcher instead refuses an existing process, but substituting its different suite source silently would defeat exact-source qualification. No launcher was executed or changed.

MEASURED — trusted host whole-boot kernel journal contains 533 historical Xid records at preflight. CODE READ — `codex/sweep-ada-20261022:scripts/bench/native_xid_guard.py` requires exactly five source-matched native faults (`approve_native`, lines 82–96), and later compares against the whole-boot kernel snapshot (`kernel_snapshot`, lines 399–407; `supervise_guest`, lines 341–346). Its unchanged path cannot accept this host history as a new five-fault baseline. This is a preflight compatibility finding, not an executed guard refusal or a newly observed candidate Xid. No records were removed, no allowlist broadened, and no cursor workaround was implemented.

CODE READ — binding stop instruction: “If something contradicts the code or these instructions, STOP and write down options; never improvise a design change.” Also: “If a step is blocked or refused by your own safety checks, STOP and report; do not reword or work around it.” Q1 stopped hardware preparation and launched no further qualification commands. The previously started local full-suite command is allowed to terminate; final counts follow.

INFERRED options for owner: (1, recommended) authorize a separately reviewed, tests-first harness change that proves process ownership and preserves an exact pre-run journal prefix while admitting only the five fresh native and exact guest control faults; re-pin its source and resume Q1; (2) authorize an explicit approved-sweep-runner qualification profile with candidate product provenance kept separate, plus a defined journal epoch; (3) retain candidate on its integration branch, unqualified. A host reboot alone addresses journal history but not launcher ownership. None of these options has been executed.

MEASURED — candidate source was staged on the trusted host, exact head verified, but `build_kf3.sh` was never launched. Native gates, Linux suite, Windows CoW/benchmarks, production-log comparison, clippy/rustfmt and mutation checks are NOT DONE. No product behavior change was made.

MEASURED: candidate ancestry contains e64b8e5f6 and the descriptor, SA-02 and SA-03/VA-2 merges f717a3422 / 7cbacd778 / f7cf3ed81. Candidate source staging and the first evidence push were verified; only Q1 evidence branch was pushed. H1 remains incomplete because CI is queued. H3/H4/H5 were not run after the safety stop.

MEASURED: locked final host-health check at 07:06 CEST returned cuInit 0 before/after, no reload, NVIDIA bound, GDM active and successful nvidia-smi. The GPU lock was released and immediately reacquirable; no host target directory exists. See restoration.json.
