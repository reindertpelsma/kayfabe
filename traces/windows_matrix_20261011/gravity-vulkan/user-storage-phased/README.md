# Native phased private storage

**STATUS: RESEARCH, 2026-10-12 03:36UTC.** No result yet.

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
