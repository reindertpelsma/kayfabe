# Windows kernel UVM routing correction

**STATUS: RESEARCH, 2026-10-11.** Product change, hardware verification pending.

Fresh old-product evidence is in `../encoder-session/managed-fresh/`. Kernel
UVM migration channels inherit the application's PID and were classified as
passthrough user work. Physical copy0x3185 is correctly rejected by the USER
host channel. Keep KERNEL+UVM_OWNED declarations on the existing authored
Translated route at birth, before applying the Windows per-process shortcut.

UVM_OWNED alone never supplies the KERNEL stamp or privileged T-space witness.
The host stays USER, its operands are authored into validated VM windows, and
ordinary Windows user-work channels retain their prior route. No timeout,
completion forgery, per-die constant or family exception was added.

GPU-free: kf-rm + kf-qemu898 tests PASS. The regression checks the actual failure
facts and the unprivileged UVM counterexample; the existing process-rule grid
also requires every UVM-owned variant to decline passthrough reclassification.
Formatting was applied only to the two changed source files; pre-existing
format differences elsewhere are excluded.

Hypothesis for hardware: keeping these kernel UVM channels Translated removes
the physical-copy refusal and managed allocation/visibility tests complete.
Falsifier: an allocation still hangs, any translated birth/operand refuses, a
fresh host Xid appears, or the existing Linux gates/fast suite regresses.
No supported managed-memory claim until the original180s oracle passes cleanly.
