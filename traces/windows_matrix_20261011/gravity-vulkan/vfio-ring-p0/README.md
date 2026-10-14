# Native allocation parameters for Windows ring migration

**STATUS: RESEARCH, 2026-10-12.** Native allocation capture and Gravity reference complete; cursor transfer remains unmeasured.

H-ALLOC-OBS and its falsifier precede the run in the restricted design review.
The former reference saved only the fn103 allocation prefix. Public source shows
a32-byte allocation body prefix and full committed transport-element copies;
observer now captures only declared params within those validated stable pages.
Flag8 identifies params outside the RPC checksum; updated decoder keeps checksum
coverage separate from stable snapshots/export hash. Oversized claims keep only
prefix. Bounds/core GCC+Clang ASAN/UBSAN/decoder tests PASS; red/green logs here.

A fresh original-baseline child, original600s Gravity workload/pinned72741e2b
matrix oracle/driver580.88/RTX4070. Personal1380 untouched. New standalone QEMU
built from10.2.4 tar plus public observer; no refusal/driver-analysis patches.
Capture window300s from each boot, same fixed source/output bounds. A limit is
reported as missing coverage, never absence of migration. Input key events are
not traced. Do not archive guest password, RAM, disk or environment. No native
cursor rule or Windows product compatibility claim follows from alloc params.

Initial5a407dcd8 build completed; writer unit test lacked explicit libqom.a target,
so it exited before GPU detach/VM launch. Cleanup also found rg unavailable on
host (kernel query saved, filter unavailable). EXIT1 is a harness failure/Windows
NOT_RUN. New immutable retry runner uses the same compiled native binary, verifies
all observer sources byte-identical, builds that test library and uses grep.
Results stay separate in retry-1; source/runner provenance recorded separately.

## Measured result, 2026-10-12 01:07UTC

Native QEMU source5a407dcd89d82f1a63520ea5086145e8fe4d08c1; immutable retry
runnerf15da21bcb7439fd3a8eab88269af1ebf8796f13. QEMU SHA256
1325fefb009cb2b30a2247d4114a73bbe1652f5e2d6c57884187834b2246afa3.
Same RTX4070/Windows580.88, fresh disposable baseline child,8vCPU/8GiB, native
12GiB VRAM. Gravity Vulkan **PASS183s/rc0**: image859468bytes,128652 timing lines,
NVIDIA3D98.67%/copy39.43%, peak2248MiB, zero153/4101/WER/live-kernel/reboot.
Observer GSP-cycle count is unmeasured. Shutdown completed without forced kill;
wrapperEXIT0, NVIDIA restored, DMA-FQ/noQEMU/lockfree/stop-file absent,
123491311616bytesfree. Exact bounded kernel query succeeded, zeroXid. The restore
script's xid=43 is its historical global count, not a new run fault.

Strict decoder validates18923 records, complete export/footer/source and metadata
hashes, no declared sequence gaps/drops. **Passive capture is not complete:**
limited=true at300s,142 unstable snapshots,6818590 invalid-element attempts.
Those are observer counters, not guest GPU faults. All65 observed C56F allocation
requests have complete368-byte params. Allocation-tail provenance remains outside
the RPC checksum, within stable committed transport elements and export hashes.

Offline audit uses compiler-produced public580.65.06 layout and public595.84
RPC memory/free declarations. Four FIFO-update registration replies are successful
for clientc1d0007a/channelff040026/API4000002b..2e. Original birth:32768 entries,
GPFIFO0x6b20000, USERDff00a004/offset0x800, process5096. Requests are:

| API | entries requested | GPFIFO VA | USERD handle / offset | observed memory candidate |
|---|---:|---|---|---|
|4000002b|65536|0xa1c2c0000|ff00a00b /0x600|SYSTEM list,4096bytes,1page|
|4000002c|131072|0x2dfe90000|ff00a00c /0x800|SYSTEM list,4096bytes,1page|
|4000002d|262144|0x1fe00000|ff00a00b /0x600|SYSTEM list,4096bytes,1page|
|4000002e|524288|0x2ffaa0000|ff00a00c /0x800|SYSTEM list,4096bytes,1page|

These measurements confirm both ring growth/relocation and different USERD handles
and slots versus birth. Four registrations here versus two in the earlier passing
reference is an observed variation; identical migration count is not claimed.
Candidates honor observed free barriers/handle reuse but are **not ownership pins
or a complete lifetime proof**. Each comparison is against birth: do not assume
that any preceding registration executed. Actual GPU trigger, old/new cursor
continuity and safe old backing retirement remain UNMEASURED. H-ALLOC-OBS's stated
falsifier did not occur; this is not a Kayfabe app pass or a cursor rule.

Raw capture and original logs are compressed in result/. Reproduce allocation
summary by decompressing gsp.jsonl.gz then running audit_allocations.py on it.
No disk, guest RAM, password, input keys or environment is archived.
