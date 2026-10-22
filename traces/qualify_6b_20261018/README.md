# Q4 candidate-6b: FAIL; owner-yield checkpoint

CODE READ: candidate revision baa2977527c4509145e8033143bd664f768175a6 is pinned, with SA-02 excluded. This evidence-only lane changes no product code. The owner requested the host after the four fresh Energy trials and before any further long SPECviewperf or smoke arm. Q4 must wait for Claude's explicit return message before more GPU/VM work.

MEASURED on 2026-10-22: [run 4400](4400/result.json) failed during Energy-04 after Blender, CATIA and Creo completed. There were three nvlddmkm event-153 records, zero event-4101 records, one poisoned channel and one NoCeObject refusal event (three textual mentions). The original nine-workload/two-iteration attempt is a failure, not a completed pair.

MEASURED on 2026-10-22: the four additional fresh Energy trials [4401](4401/result.json), [4402](4402/result.json), [4403](4403/result.json), [4404](4404/result.json) had one failure and three passes. E1 recorded three event-153 entries and one poisoned channel; E2/E3/E4 completed with zero guest 153/4101 events. The passing scores describe survivors, not unconditional stability.

MEASURED on 2026-10-22: all five Windows arms 4400–4404 had zero new host Xids, zero Budget diagnostics and zero PROGRESS_STRAND lines; all restored to cuInit 0 without a module reload. Run 4400 exercised large descriptors: maximum retired-channel large-entry count 43,376 and maximum descriptor size 1,043,244 bytes. Each fresh Energy arm used a separate CoW overlay of the owner backing, whose size and mtime were unchanged.

INFERRED: do not merge this candidate. The runtime failure also occurs without SA-02; the earlier small-sample rates do not identify a sole cause. Q4 makes no speculative product fix.

MEASURED on 2026-10-22: [candidate-tests.json](candidate-tests.json) records 3,781 passed, zero failed and 20 ignored across the full 22-crate GPU-free release suite. [clippy-delta.json](clippy-delta.json) records 532 warning sites in both candidate and master, zero added and zero removed; both commands succeeded.

MEASURED on 2026-10-22: [rustfmt.txt](rustfmt.txt) records a formatting failure in the added all-PDB security test. [ci-final.json](ci-final.json) records hosted run 39764911164 failed at the same Format check; stable tests and Clippy passed, while later boundary/claims steps were skipped. Formatting remains a merge blocker.

MEASURED on 2026-10-22: [broker-check.json](broker-check.json) records the full local broker check passing, including all 29 XOR and 28 tick mutants killed by at least two distinct checks each. These checks do not establish host-visible broker usability.

MEASURED on 2026-10-22: [native.json](native.json) records 9/9 native gates plus gate 10 passing, 12/12 USER births, zero refusals and zero new Xids. [linux.json](linux.json) records 30/30 fast-suite arms, exactly the approved four defer-control and one missing-page-control Xids, and zero unmatched new Xids. The initial wrapper-mode packaging failure did not boot QEMU; the corrected complete retry passed.

MEASURED on 2026-10-22: [host-display-4400.json](host-display-4400.json) records an active, unlocked host GNOME session reporting zero monitors despite two connected NVIDIA outputs. Broker frames were discarded with no presentation feedback in every Windows arm. A pointer wake did not restore presentation. Cursor hover/grab, visible input and broker performance remain unverified. The GDM-restart own-PIDs exception was asked and remains unanswered; no display-only restart was performed.

MEASURED on 2026-10-22: [comparison.md](comparison.md) and [comparison.json](comparison.json) preserve valid vendor-completed composites and ratios against the available K1 master and VFIO references. The master baseline has only Blender completed; missing baseline rows are unavailable. Historical references were not interleaved with Q4. No percentile broker lows are inferred from threshold-only stats. Each arm result also retains the independent viewperf exit census; the observer does not identify who killed a process, so a forced-kill count remains unverified.

CODE READ: Windows used run-3440 production flags minus KF3_PROF, KF3_DIAG_PROGRESS=1000, optional lifecycle counts, fresh CoW overlays, and the display broker built from master revision e9c6d39e5b465d7a20f3dcca8e7e15a4374958b5 with optional --stats-line output. Actual candidate build and broker hashes are in native.json and the harness manifest.

CODE READ: deferred until the host is returned: separate eight other SPECviewperf workloads twice, Heaven D3D11 windowed, Minecraft Java ten minutes, Steel Nomad GUI, broker hover/grab cursor and input, and final qualification reporting. app-inventory.json found no app-matrix PowerShell harness and no 3DMark command-line executable; Steel Nomad is installed. No smoke arm was started. The separate eight-workload arm cannot turn the failed original full-suite verdict green.

CODE READ: HOST_YIELDED, final-health.json and yield-audit.json record the handoff. Q4's own target directories are removed, workload processes stopped and fastguest lock released. The visible-desktop limitation above remains disclosed. The original tool-filter stopped checkpoints are superseded history; the original seven-hour deadline is 17:41:44 UTC unless the owner changes it.
