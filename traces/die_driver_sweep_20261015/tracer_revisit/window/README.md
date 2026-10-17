# Early-window follow-up

**STATUS: LIVE, 2026-10-16. CPU/WDK PASS; native H-BC4 PENDING.**

Source631f087e34e2e575f303835da9f11fedd3a97df6, same draftPR14. Failing-first
dd90f9c53 compiles33/35tests pass with3failedassertions. Initialimplementation
35/35passed; addedRAM-edgefixture failed35/36 becauseRAM_BYTES was incorrectly
treatedasaphysicalend. Commit1e34bd8ce prematurely claimed36pass;631f087e3
corrects thatclaim andthefixture without rewritinghistory. Corrected36/36,
393checks0fail. FullharnessALLSTEPSPASS (threadstress,23mutations,221roundtrip,
bothcensusoracles,WDK/PE/import/read-onlyguards,forbiddenstorecompilerejection).

UnsignedmoduleSHA1b0a549b7df5c9472dc6842eaf80e149fe7ec02a720fb9d874bade987413f1c6;
collectorunchangedb7732d2e...7acf. Internal sameGPUcache only, oneGiBforward
window for15seconds, <=oneMiB perstep, <=16steps beforeyield. Existingcomplete
region/checksum/empty-or-seq0 guards required;method4distinct. Ordinaryscan/
backoff resumes afterdeadline. No ioctladdress,newMMIO,GPUstore, runtimeRM
reply or capturedper-diefact. This is a heuristic whose nativefalsifier remains
nonzero prefix/loss/noearlybind/stalequeue/appfailure.
