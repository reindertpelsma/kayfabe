# Read-only GSP queue revisit validation

**STATUS: LIVE, 2026-10-16. CPU/WDK PASS; native proof PENDING.**

Smallcodebranchcodex/windows-capture-revisit1d302de82, draftPR14 against
claude/gpu-types-20261015. Failing-firstfc7e8b862 compiles,29/31tests pass,
6assertions fail; after33/33tests,381checks,0fail. Fullcorrectedharness passes
ringASAN+TSAN51checks each,metadata21,read-only guard+self-test,23mutations
allcaught,221roundtripchecks,2actualcensusoraclePASS,lossy87messages rejected,
WDK/PE/importguard and negative forbidden-store compile. First full harness
attempt had missingcargoPATH, causing3dependent fixture/roundtrip/comparison
failures. Corrected PATH and own isolated fixture paths; no product change.

UnsignedmoduleSHA62e89c6dd256ec56c4b951c1d40faa485d1e2919756b36bda0449ca1f539f530;
collectorSHAb7732d2edb0845f9d319d0dd2669474ff7cb7bec1adc54c10f261bd97c427acf.
Trustedbuild/data/codex/build-tracer-revisit, MicrosoftWDK/SDK10.0.28000.2526.
NativeH-BC3 plan inblackwell_5060/native_trace_revisit_20261016/PLAN.md. CPU
models do not prove physical allocator reuse, no dropped native prefix or
native module load. Require method3/seq0bothdirs/loss0/meaningfulboot+probe
traffic and strictnativeCUDA/D3D before a complete-reference claim.
