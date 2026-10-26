# GA104 native baseline

STATUS: LIVE, 2026-10-20. MEASURED PASS at e8553e6ce7ec557aef9ff96c3c876e3060a6b9f8,
stock open host595.91.07, host-derived GA104/0x170/0x4, PCI2484.
Strict health passed before gates9/9+gate10/USER12/12; native raw30/30 and
CUDA4/4 passed at00:19:28..00:20:53UTC before the first guest raw run.
QEMU SHA256 b33394dd23e515994a4478747a52311d05f1a1bffbb300eb4415f2a0e22220ef.

native_cuda595.txt is the canonical exactly-four-rung table that the native
controller graded. All four rows have verdictPASS/rc0 and required copy,
kernel43, matrix-error and timed-iteration content checks. n1/spreadN/A.
native_cuda595_cli.txt preserves the CLI stdout, which repeats the first three
rows before printing the full table. Those seven row occurrences are not seven
rungs or repetitions.

The first export assertion incorrectly required four rows in the CLI stdout;
it failed, but that publication shell continued. The correction preserves the
stdout separately, exports the already-graded canonical table, verifies all nine
artifact hashes and all four content checks with shell stop-on-error enabled.
No GPU rerun or grader change occurred. Original failure and correction are in
commits b37ce56dc and1200cb466. Source/export hashes and GPU-identifier redaction
are explicit in export_receipt.json. No complete guest acceptance is implied.
