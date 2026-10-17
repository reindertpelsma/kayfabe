# H-FMC1: observe the public FMC mailbox indirection

**STATUS: RESEARCH, 2026-10-16 00:58 UTC. Written before hardware replay.**

The first virtual observer exported no messages: one invalid bootstrap, zero
attached tables. Public OGKM580.65.06 `kgspBootstrapGspFmc_GH100` puts
GSP_FMC_BOOT_PARAMS in MAILBOX0/1; gspifpub.h compiler output locates
gspRmParams.target at 40 and bootArgsOffset at 48. Kayfabe already follows
this indirection in its FSP boot implementation. The shared observer only
tried direct LibOS arguments.

Hypothesis: trying exactly one system-memory FMC indirection after the direct
route fails will attach the virtual queues before either sequence-zero prefix
is overwritten. Preserve every existing RAM/table/queue validator. No scans,
recursive walks, local-FB reads, guest writes or runtime replies change.

Falsifier: no attachment, either prefix missing, gaps or decoder rejection.
Attachment alone is insufficient. If falsified, preserve the result and rank
the next trigger/ordering candidate; stop before this rental's existing timer
01:39:19 UTC. No further runtime controls will be guessed from this run.

Clean code branch codex/observer-fmc d18aafad4 (failing first ade898857);
CPU GCC+Clang ASan/UBSan and decoder tests pass, pinned compiler layout checks
pass. Integrated runtime 3934a0895 differs from prior 72fca168a only in this
diagnostic observer and its tests/docs. Guest actual expected 32.0.15.8088,
public ABI580.65.06; host595.91.07. Actual process receipt must be asserted.
