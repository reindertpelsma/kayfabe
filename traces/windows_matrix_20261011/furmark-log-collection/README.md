# Fresh FurMark log collection

**STATUS: RESEARCH, 2026-10-11.** Hardware pending. Product remains72741e2b. The normal stress run completed65s/rc0 with visible NVIDIA renderer but failed its existing proof: all sampled PDH activity0, no stdout renderer identity. Original FAIL retained in managed-routing/remaining-apps.

The old collector chooses the first root *.log. It does not establish that this is FurMark's renderer log. Bench correction adds the documented --logfile-suffix with a unique per-run GUID, then searches at most16 matching names and the last256 lines per file. It neither assumes an extension nor admits another run's renderer text. Missing identity still fails the existing oracle. App/duration/rendering settings and thresholds are unchanged. Collection correctness and a hardware pass remain pending.

Primary artifact: pinned FurMark_2.10.2_win64.zip from manifest.json, SHA25627ab2e723e2e65df720bcafea681d2104744eda4a1e0a0374d7e61eaa820e63b. Its help.txt documents the logfile-suffix option; it was inspected as text on the controller, no executable run. No binary analysis.

Validation: existing76-test suite had75PASS and one generated-document staleness failure. Regenerated documentation, then all11 inventory/document testsPASS, including generated app-script parsing. Product unchanged; no new product hardware claim.
