# Fresh nbody GUI retry and setup failure

**STATUS: RESEARCH, 2026-10-11.** Product72741e2b, bench738b5385.
The first stream row is BOOT_FAIL only: the partial source archive lacked the
sign-in helper; no stream app ran. Full frozen archive was completed. The
harness moves on after boot failure; fresh stream1350+ is queued separately.

Nbody GUI1341 TIMEOUT192s(supervisor deadline, canonical app90s plus harness
slack), one GSPcycle, recorded guest driver/WER counters0. Host Xid32 at
17:04:46UTC/QEMU155065/channel0x4a/intr00800000. Matching stock GUI PASS is in
reference-unigine-nbody. The unrelated older numerical diagnostic fails on stock
also and supplies no replacement oracle.

Channel0x4a was reused before the app. The second birth is guest
c1d00070:ff040021, passthrough on GR engine1, host deferred-API objectcafe0621
exists but has no host registered actions. Guest registration0x50800104 names
FIFO_UPDATE_CHANNEL_INFO0x20801116,584-byte outer registration. This is
correlation, not proof that a particular trigger caused the fault. No faulting
method was measured. Full source hash and bounded channel excerpts saved.
Public GA100 PBDMA intr0bit23 names DEVICE/SW-method; its applicability to AD104
is inference. No CPU completion or unsafe host registration was introduced.
