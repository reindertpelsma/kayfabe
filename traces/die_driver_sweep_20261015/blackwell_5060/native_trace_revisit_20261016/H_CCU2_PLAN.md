# H-CCU2: derive sample metadata through an unprivileged public host API

**STATUS: RESEARCH, 2026-10-16. Written before host query.**

NativeWindows580 CCUsampleoracle is17296bytes, notregistered/shipped. Public
595.91.07commit9f087a6d4e86d85acc0ce1d354d6276fe5047b29 has the sameGB100
formula as580.65.06: devBufSize=2*(ccuSampleSize-sizeof(NvU64)). Public
KernelCcuApiSUBSCRIBE returnscounterBlockSize=devBufSize for non-MIG, and
bufferSize; itsnon-MIG implementation readsmetadata, doesnotmaporstartstream.
Generated580HAL selectsGB100GetBufSize forGB206.

Hypothesis: allocate publicNV_COUNTER_COLLECTION_UNIT0xcbca under the owned
hostGPU, query publicSUBSCRIBE0xcbca0101 withcompiler-derived16Bshape, read
onlycounterBlockSize/bufferSize, freeobject. Run UIDnobody/CapEff0 on595.91.07,
no streamstate or map/read ofcounterdata. Inversecandidate=counterBlockSize/2
+sizeof(NvU64). Compare against thenative580oracle; nevercopyoracleintoproduct.

Falsifier: alloc/controlrefused orwrongparent, missing/odd/invalidsize, or inverse
notmatchingnative580oracle means this hasnot establisheda cross-ABIfact. Wrong
parent mustbe distinguishedfromlackofAPI support. Do notretryasroot orloosen
profilingpermissions. No capturedsamplesizeconstant, firmwarebinaryRE or GPU
workcompletion. Query isseparatefromandbefore anyguestbenchmark; noparallelGPUjob.
