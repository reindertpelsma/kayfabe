# Handoff inputs, 2026-10-04

**STATUS: ARCHIVE OF INPUTS, 2026-10-04.** These are the machine-readable results of the review and
design workflows run on 2026-10-03/04. Until now they existed only in a session scratch directory.
They are committed so work can resume from the repo alone, by any agent or person. Each one is
superseded by the design doc on its lane's branch as soon as that branch folds it in. Read the lane's
branch first.

| file | what it is | lane / branch |
|---|---|---|
| `cand1_result.json` | candidate 1: merge, merge review, the full real-GPU test (merge bar, apps, display), and the B5 failure | `v3-cand-1` |
| `sec_p0p1p2_design.json` | the P0 privilege fix, the P1+P2 design (identity windows; the Translated space) and their adversarial reviews | `v3-sec-nonpriv`, `v3-p1p2` |
| `s1_21_verify.json` | verification of audit S1-21 (windows in user twin spaces), with three refutation attempts | `v3-p1p2` |
| `fps_design.json` + `fps_owner_decisions.md` | the display-max-fps design (`OWNER_RULINGS` §M) and the owner's decisions D1-D5 | `v3-maxfps` |
| `broker_r2.json` | broker round 2: relay vs nvkvm-pv `badf2d7`, XOR blend, console cursor, box results, 16 review fixes | `v3-broker` |
| `gpucopy_design.json` | the GPU-copy rung design (§L) and its adversarial corrections | `v3-broker` |
| `viommu_result.json` | guest-IOVA readiness: site table, seam, detection, OD-1..OD-6 | `v3-viommu` |
| `p1p2_result.json` | the S1-21 fix: revised design, two reviews, implementation, fix round, box plan, owner decisions 8-12 | `v3-p1p2` |
| `rawaddr_result.json` | S1-03/S1-04: design, reviews, implementation and fixes | `v3-sec-rawaddr` |
| `dispsw_default_on_prep.json` | prep for x11-dispsw default-on: forced-release probe design, client-split inventory, checked plan | (not started; owner question §N cond. 3) |

No secrets, keys, addresses of machines or binaries are in these files (checked before commit).
