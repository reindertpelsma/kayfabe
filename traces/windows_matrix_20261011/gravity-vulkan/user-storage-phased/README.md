# Native phased private storage

**STATUS: RESEARCH, 2026-10-12 03:39UTC.** Native exact7135568d3 PASS; no VM wiring.

Hypothesis: splitting private storage initialization into prepare, bounded4KiB
clear pieces and a later read-only map preserves the native CONTROL NOP WAIT
consumption-barrier result without one phase exceeding the12ms invalidate bound.
Falsifier, stated before run: any phase >12000us, incomplete initialization,
copy/barrier criterion failure, host Xid, failed native channel/storage cleanup,
or probe timeout. One variable versus exact6e60662d1 native PASS: initialization
phasing. Same RTX4070 native595.91.07, USER C56F/uid65534, same real CE copy and
explicit held-acquire/release stimulus. No VM and no Windows migration proof.

GPU-free kf-chan/kf-qemu/kf-rm suite passed; final kf-qemu lib169/169 after the
full-span handover correction. Independent source review found no blocker for
this isolated native timing probe; runtime wiring still requires birth identity,
successful-native-free-only teardown, exact Linux and Windows validation.
No timeout or phase observation grants resource/completion/reuse credit.

## Measured result

Exact7135568d3dc11c5a0af7ba6151ce74c760c340b4, native RTX4070/595.91.07,
USER C56F/uid65534, binary SHA in result/source.json. Prepare66us;256 clear
pieces, maximum115us; read-only map78us. Held48 samples/maxGET1/reference0/
CEsem0; explicit release then real4096-byte copy/reference1/CEsem1/GET4/PUT4,
52899us. Probe/runner EXIT0, bounded hostXid0, native channel/storage/fixture
cleanup successful, no forced kill. Falsifier not observed in this one run.
Native phase measurements do not establish a worst-case latency guarantee.

Post-run: NVIDIA restored/DMA-FQ/noQEMU/lockfree/stop absent,
120306757632bytes free; personal image28240642048bytes/mode0444 unchanged.
Original text-only probe/verdict/source/post-host records and hashes in result/.
No guest image, RAM, password or private-driver material exported. Windows
migration, channel-birth/cancellation identity and exact Linux regression remain
open; this is not a GravityMark or matrix pass.
