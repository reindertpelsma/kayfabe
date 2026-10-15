# Turing display semaphore wall

**STATUS: RESEARCH, 2026-10-15.** The earlier TU106 rental refused `display=on`
at `NVC57E_SET_SEMAPHORE_CONTROL_PAYLOAD_SIZE` (die-sweep §3.4). This is a
measured code/vocabulary failure, independent of Windows' later initialization.

H-T1, stated before the test: select the shared 32-bit acquire/release model
from the generated class vocabulary, and Turing's display realize path resolves.
Falsifier: either complete-realize test still fails, or an engine test fails to
wait for the shared acquire / release the outgoing flip to the same slot.

Public derivation: NVIDIA ogkm 580.159.04, commit
`b81d58ee0224d1d290bef1c080592b619e184042`, `clc57e.h` semaphore methods;
`nvkms-evo3.c:EvoProgramSemaphore3` programs one context DMA and offset,
32-bit acquire and release values, and disables both with context DMA zero.
The source's head-surface acquire check compares equality. No per-die value is added.

This is a candidate fix. Hardware acceptance and Linux broker regression are
required before any claim that Turing display works.
