1. CODE READ (2026-10-22): ROUND 2, laneS4, branch `codex/sweep-dies-20261022`; EXPECTED RED commit `3de4e17b02095869175cb6ed979a02b86450b580`; final publication head is the containing commit, verified on origin at closeout.
2. CODE READ (`round2/DERIVATION_BLOCKER.md`): asked to derive the fault client from public OGKM plus host engine facts, then resume Ampere and Turing; result BLOCKED at the public derivation, before any rental.
3. CODE READ (`scripts/bench/native_xid_guard.py:66,70,327`): CE1 literals found in native and guest admission; COPY0 defer controls intentionally provoke four read faults, plus one separate GR missing-page fault.
4. MEASURED (lease57209908, `native_controls/pce-mask-probe.log`): COPY0/LCE0 host mask0x10; four journal faults reportCE0; this observation cannot independently justify admitting that same journal.
5. CODE READ (`round2/sources.json`): public OGKM580.126.09,595.91.07,615.78.08 inspected, with immutable commits, archive hashes and CE/FIFO/UVM source hashes retained.
6. CODE READ (`round2/*.search.json`): C/H searches covered3694/3708/3914 files; no independent PCE-to-HUB-client rule was established; regex absence is not proof of global absence.
7. CODE READ (`round2/DERIVATION_BLOCKER.md`): missing fact is the public generation-specific bridge from GA102 backingPCE4 to its MMU HUB client; fault-name enums and LCE/PCE masks describe separate identifiers.
8. CODE READ (`round2/facts_not_admission.json`): evidence classes separated; no inferred admission table, CE0/CE1 allowlist, guard change, product change, timer change or baseline expansion.
9. MEASURED (`round2/round2_expected_red.log`):8tests,6PASS,2EXPECTED RED; authentic AD102 exact-five replayPASS; authentic GA102 rejected, while a CE1-substituted journal is admitted by the existing guard.
10. MEASURED (`round2/round2_test_*xid*.log`): existing native18/18,guest61/61,abort5/5PASS; extra-record, wrong-channel, wrong-VA and unrelated-Xid refusals retained; after-fix proofNOTDONE.
11. CODE READ (`round2/DERIVATION_BLOCKER.md`): AD104 paired replayNOTDONE, not relabelled from AD106; derivation tests/fix/mutantsNOTDONE; Rust changes0, added Rust lint sites0, clippyNOTRUN.
12. CODE READ (`round2/DERIVATION_BLOCKER.md`): Ampere Windows/bind/ON/appmatrix/Unigine/600-second hold and TuringNOTRUN; ROUND2 explicitly requires stopping when public derivation cannot be established.
13. INFERRED decision1: recommend obtaining the missing published PCE/client mapping before another lease; no further permission question or speculative mapping was substituted.
14. MEASURED (`round2/cleanup.json`): own ids57193877/57209908 API-absent; new leases0, Round2 rental cost$0; INFERRED conservative lane accounting$3.80362 remains below the Round2$8 stop.
15. MEASURED (`round2/cleanup.json`): owned helper jobs0,target absent, GPU/cargo locks acquired0; claims scoped to lane changes are checked before publication; no private RE or credentials used.
