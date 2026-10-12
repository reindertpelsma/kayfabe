# Native C56F MAIN GP-entry packet continuation

**STATUS: RESEARCH, 2026-10-12.** Hypothesis/falsifier written before hardware.

H-CONTINUATION: a C56F USER PBDMA continues a pending packet's arguments across
adjacent direct MAIN GP entries. Same read-only commands/unmapped private USERD,
virtual4096-byte CE copy, WFI_ALL and SET_REFERENCE as the previous positive.
One variable: GP0 ends at the final SET_REFERENCE header and GP1 holds its sole
argument. No command bytes, mapping permissions or engine parameters change.

Falsifier: identical unsplit control passes, but split reference/copy/semaphore
or GP_GET/PUT2 does not arrive, a new Xid occurs or channel teardown fails. Run
control, then split, then same split under the existing explicit50ms acquire hold.
The held case additionally requires no reference before the explicit signal.
The observation deadline stops collection only; channel free must succeed before
resource revoke. No app/default/VM behavior or source lifetime proof is supplied.

Run unprivileged uid65534, shared GPU lock, one process at a time, trusted host,
no QEMU/personal Windows image. Source/SHA, PID/UID, bounded journal, start/exit
and actual GPU facts retained. Stop after first failure. Parser's concatenation
model is not called a native result until these assertions pass.


## Measured result, 2026-10-12 01:54UTC

Exacted5c3ec0892c712328f5800a652354b62fbee85c, RTX4070/host595.91.07,
uid65534 USER C56F. Unsplit PASS2267us/GET1/PUT1; split PASS2254us/GET2/PUT2;
split-held PASS52682us/GET2/PUT2. All real references1, correct4096-byte CE copy,
CE semaphore1, no hostXid, clean native teardown. Held47 samples saw no early
reference or copy; after explicit signal they arrived. No VM/personal disk used.
H-CONTINUATION's falsifier did not occur for this MAIN direct-entry case.
This validates continuation of the final reference packet only, not every engine,
SYNC_WAIT/control form, migration cursor or command-backing lifetime.

Initial host build launch lacked cargo in PATH; explicit cargo path then caught
an overbroad edit in the older non-reference mode. Both failures preceded GPU
launch. Corrected exacted5 tests/build pass; initial mismatched full revision
argument was refused before root creation/GPU work. All failure evidence is
retained; only the corrected exact revision supplies hardware results.

Original probe/journal/verdict/source/hash are in result/. Native binary SHA:
db7388cf062cc8c15b528b9a0dba5a3c17dd06ddfd4599a86484933573e07b3f.
