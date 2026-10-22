# Q1 candidate-6 qualification

STATUS: IN PROGRESS, 2026-10-22.

MEASURED: candidate f7cf3ed81200077e55ee7769c635887661f3f990 fetched from origin. Q1 evidence branch: codex/qualify-cand6-20261022. Start 05:01 UTC; hard host-restoration deadline 10:00 UTC. No product edits authorized by this qualification.

INFERRED H1: the candidate is the stated three-fix integration. FALSIFIER (before checking): different remote head, absent fix ancestry, or failed required CI job. MEASURED: remote head agrees; CI run 39725599154 was queued at first check. CI and ancestry result pending.

INFERRED H2: all candidate GPU-free suites remain green and clippy adds zero diagnostics versus e64b8e5f6. FALSIFIER (before execution): any failed test, rustfmt failure, or added source-bound clippy diagnostic. Result pending.

INFERRED H3: merged regression tests discriminate each fix. FALSIFIER (before mutation): restoring the old descriptor budget, handle privilege authorization, or >64-space refusal kills fewer than two tests. Result pending.

INFERRED H4: candidate passes native 9+10 and Linux 30/30 under the approved exact-match intentional-fault policy. FALSIFIER (before execution): failed/missing verdict, non-USER birth, missing completion, or any new unmatched host Xid. Result pending.

INFERRED H5: all nine SPECviewperf workloads and available production regression smokes complete on the candidate. FALSIFIER (before execution): benchmark failure/kill, guest nvlddmkm 153 or Display 4101, new unmatched host Xid, or descriptor/scan budget refusal. Require exercised large-Pointer counters. Result pending.

CODE READ: lane requires native-before-guest, candidate-built binary, independent Windows CoW, production flags, pinned K1 runner, and no master/candidate push. Raw logs stay outside git. Measurements and limitations will be appended here.
