# Exact Windows581.51 display prerequisite

**STATUS: LIVE, 2026-10-16 21:34UTC. CPU PASS; hardware UNRUN.**

Public580.94.02 commita86526f4e235ae3d9e2842a11a3bb11db77450d8, VK580_65-5/CL36629274
identifies Windows581.51. Three generated display tables7528rows compiled from public headers;
clean layouts generator reproduces them byte-for-byte. PR31test6e5f08a81 fails absent lookup,
fix98b49a1a4 passes191kf-disp CPU tests/2ignored, including all4family pages withoutmeasuredrows
and adjacent-tag refusal. No whole-page oracle, neighboring ABI or per-die wildcard.
OwnGB205integration9d6f210ba additionally includes independent580/595 word0x00c=0x102 rows,
206displayCPU testsPASS/2ignored. The first integrated check failed2stale unknown-neighbor
cases afterGB205 gaineditsown row; they now use unmeasuredGB203 and keep provenance/refusal tests.
NativeWindows8151binding/signatures, completeGSP and allhardware suites remainUNRUN.
