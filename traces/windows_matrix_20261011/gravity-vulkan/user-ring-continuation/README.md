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
