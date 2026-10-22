# Q4 candidate-6b qualification: in progress

CODE READ: candidate revision baa2977527c4509145e8033143bd664f768175a6 is pinned. The owner resumed Q4 on 2026-10-22 and explicitly exempted tool-runner command-filter rejections from the stop rule. The original stop.json and stopped report are superseded checkpoints, retained as history. No qualification verdict is claimed yet.

MEASURED on 2026-10-22: traces/qualify_6b_20261018/candidate-tests.json records 3,781 passing tests, zero failed and 20 ignored across the full 22-crate kf-* release suite. traces/qualify_6b_20261018/clippy-delta.json records 532 warning sites in candidate and master, with zero added or removed. Both Clippy commands succeeded. traces/qualify_6b_20261018/rustfmt.txt records a formatting failure confined to the added all-PDB security test; the candidate was not modified.

MEASURED on 2026-10-22: traces/qualify_6b_20261018/broker-check.json records the complete local broker check passing, including 29/29 XOR and 28/28 tick mutants, each killed by at least two distinct checks.

MEASURED on 2026-10-22: traces/qualify_6b_20261018/ci-final.json records GitHub run 39764911164 failed at the Format check, with the same all-PDB security-test diff as the local run. The stable job's tests and Clippy passed; later boundary/claims steps did not run. Formatting remains a merge blocker.

MEASURED on 2026-10-22: traces/qualify_6b_20261018/host-display-4400.json records an active, unlocked host GNOME session with zero logical monitors despite two connected NVIDIA outputs. All broker frames were discarded in this initial interval. A pointer wake did not restore presentation. The owner was asked for the own-PIDs exception needed for one GDM restart; benchmark work continues independently. These guest scores cannot establish visible broker usability.

MEASURED on 2026-10-22: traces/qualify_6b_20261018/native.json records candidate gates 9/9, gate 10 PASS, 12/12 USER births, zero refusals and zero new Xids. The QEMU SHA-256 and master broker revision/hash are preserved there.

MEASURED on 2026-10-22: traces/qualify_6b_20261018/linux.json records 30/30 fast-suite arms with exactly the approved four defer-control and one missing-page-control Xids, and zero unmatched new Xids. The initial packaging failure did not start QEMU; correcting the copied wrapper's executable mode allowed the complete retry.

CODE READ: Windows uses the owner-run-3440 production flags minus KF3_PROF, KF3_DIAG_PROGRESS=1000, a fresh CoW, the broker built from master revision e9c6d39e5b465d7a20f3dcca8e7e15a4374958b5, and its optional --stats-line measurement output. Planned coverage is all nine FullHD workloads twice, four additional fresh Energy trials, and regression smokes. Final CI, event/counter census, comparison table, restoration and verdict remain pending.
