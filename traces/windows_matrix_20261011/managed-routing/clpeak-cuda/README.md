# Fresh CUDA clpeak, run1310

**STATUS: LIVE, 2026-10-11.** Product/harness72741e2b, ordinary production
behavior flags, measurement flags off, MPOoff, original1500s app budget.

TIMEOUT1504s/rc124, quiet1490s, no guest driver/WER records or GSPcycle.
Output reaches tensor tf32; no later benchmark section or success exit.
Host Xid109 at18:30:02UTC, own QEMU255534/channel0x4a, CTX SWITCH TIMEOUT.
Control initiating that fault is unmeasured. Stock580.88/RTX4090 PASS906s.
No extended timeout or fabricated completion substitutes for the failure.

The supplementary kernel capture was collected after the next native test
started and includes that unrelated probe's Xid32. Keep it as raw evidence;
corrected-journal/ uses the own-run endpoint18:56:31.999999UTC and contains
only the clpeak context-switch fault. The progress screenshot at18:52UTC was
captured via QMP without input; a desktop image is not a benchmark completion.
Own VM stopped cleanly, baseline-backed overlay reclaimed, group11DMA-FQ.
