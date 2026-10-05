# Windows pool-query experiment on the borrowed RTX 4070 PC

**STATUS: RESEARCH, 2026-10-05.** Branch `codex/windows-pool-2026-10-05`.
No successful Windows-through-Kayfabe GPU workload is claimed here yet.

**2026-10-05 ABI review follow-up:** the query now checks the compiler-measured
40-byte layout and all six field offsets/widths for the exact configured tag,
instead of accepting only 580.65.06. Both 24-byte and 40-byte control envelopes
are tested across the 29 admitted guest ABIs; the measured encrypted 615 queue
remains unsupported. The timer class ID also uses the compiled matrix. Validation:
1,187 ABI/RM/chip tests passed, zero failed (`matrix-query-tests.log`). This does
not expand the hardware claim beyond the Windows/host/GPU tuple below.
The timer source audit script is only a textual spot-check, not a C semantic
proof; see the correction in the design document.

Host: native Linux 7.0.0-31-generic, NVIDIA open 595.91.07, RTX 4070 (AD104).
Windows fixture: existing 580.88 installation with GSP forced, copied from the successful
native reference. The original image is preserved; experiment runs use fresh overlays.
The physical GPU remains attached to the Linux NVIDIA driver. There is no VFIO in this test.

`initial-command.json` and `initial-driver-refusal.log` record the first attempted launch
at `c50fad9ac485f53d45d4ea77a21cb7206267c65a`. QEMU refused to realize the device because
595.91.07 was not a measured host tag. **Windows never booted in that attempt.**
The disk had already passed qemu-img check/convert/check/compare against the native fixture.

The exact 595.91.07 public OGKM tag is `26d82922dc1e444f274294505f73f1c0b8ca682d`.
The driver-matrix measurement uses the repository's four specs, GCC 15.2.0 and pyelftools
0.33. Existing tags are remeasured and compared to the committed ranges before admitting
the new tag; the version check is not bypassed.

Experiment implementation: `ae239848`. The host environment `KF3_GFX_POOL_PROBE=1`
serves only a bounded, invented virtual size query; it is disabled by default.
Two wire-codec tests and three RPC policy tests passed, including malformed lengths,
serialization, poisoned outputs and excessive slot counts. This does not validate the
physical pool formula, initialization, slot lifecycle or context binding.

Runner: `scripts/bench/windows/pc_pool_experiment.py --revision <full SHA> --name <fresh name>`;
add `--pool-probe` for the comparison run. Every run records its command and source revision.
Guest diagnostics: `scripts/bench/windows/pool_probe_status.ps1`, sent over pinned-key SSH
through a controller tunnel. SSH private keys remain on the controller.

See [the design and limits](../../docs/design/V3_WINDOWS_POOL_EXPERIMENT.md) and the native
Linux preemption evidence on research branch `codex/windows-native-trace-2026-10-04`,
commit `82fa680e`. Native mode-1 acceptance is a possible host route, not Windows support.

## Baseline A: query experiment disabled

At product revision `167fe2deaa06fd61f591bc6d2a7c43bd84da0caf`, the exact-driver guard
passes and Windows boots. NVIDIA 580.88 with both firmware registry settings equal to 1
reports Code 43; nvidia-smi exits 9. Kayfabe services 143 RPCs, refuses the pool query,
then observes the teardown and `UnloadingGuestDriver`. No GPU channels are born.
The guest shuts down cleanly and QEMU exits 0 (not a GPU success). Files are in `baseline-a/`.

The host still refuses several GSS legacy controls whose wire layouts have not been
measured on 595.91.07; these guards remain enabled in both experiment arms. A registry
listing emitted an access-denied diagnostic for the class's protected `Properties` key;
the actual NVIDIA class setting and device status were read successfully.

Validation before this run: 1112 ABI/RM tests passed, zero failures. All 124091 compared
cells for the 29 previous matrix tags were unchanged by remeasurement.

## Probe B: query experiment enabled

The same product revision `167fe2deaa06fd61f591bc6d2a7c43bd84da0caf`, on a fresh
overlay, accepts the 40-byte query with `maxSlots=4`. Windows advances past the query
to six context-property controls (`0x00801707`) and further display initialization.
Kayfabe services 209 RPCs, versus 143 in baseline A. Both logs have the query at
index 118 in the RPC trace; the two additional serviced messages are outside that
trace's coverage. No pool-initialize/add request or GPU channel birth is observed.

Windows still reports Code 43 and nvidia-smi exits 9. Immediately before the FREE
burst it tries class `0xb297`, refused with `0x56`, then `NV01_TIMER` (`0x0004`),
also refused with `0x56`. The order motivates a timer-allocation experiment; it
does **not** establish either allocation as the remaining fatal condition.
`0xb297` was not found in the public OGKM 580.65.06 tree and remains unidentified.

The source oracle for `NV01_TIMER` is OGKM 580.65.06:

- `src/nvidia/src/kernel/rmapi/resource_list.h:790-799`: `TimerApi`, one instance
  per Subdevice, no allocation parameters, unprivileged allocation allowed,
  allocation RPC routed to physical RM, no required access rights.
- `src/nvidia/src/kernel/gpu/timer/timer.c:1693-1709`: constructor returns
  `NV_OK`, destructor is empty. Allocation itself schedules no GPU work.
- `timer.c:1712-1735` and `src/common/sdk/nvidia/inc/class/cl0004.h`: register
  mapping uses the GPU's timer base and `sizeof(Nv01TimerMap)` (`0x414`).

Mapping and alarm controls have separate behavior; the empty constructor does
not justify accepting those operations without implementing them. The guest
shut down cleanly and QEMU exited 0. Raw text evidence is in `probe-b/`.

## Probe C: timer object allocation

Product revision `60d36db5ac69d756efb898940e54ecdab00b7da2`, again on a fresh
overlay with the pool experiment enabled, accepts the `NV01_TIMER` allocation.
Windows still tears down immediately afterward and reports Code 43; nvidia-smi
exits 9. One additional FREE is observed, consistent with the additional object.
There is no extra successful initialization stage or channel birth. Thus timer
allocation alone does not resolve the remaining failure. QEMU exits 0 after a
clean guest shutdown; files are in `probe-c/`.

The timer change passed 1185 ABI/RM/chip tests, including a complete comparison
with the prior capability table: exactly the timer allocation row was added at
each boundary; control decisions are unchanged. The original baseline fixture
is preserved. The source audit spans all 30 measured tags (535 through 615);
the 615 encrypted-queue transport remains intentionally unsupported.

A source-derived next hypothesis is the register mapping: Kayfabe currently
advertises `regBases[NV_REG_BASE_TIMER]=0xffffffff`. OGKM's
`gpuGetRegBaseOffset_FWCLIENT` returns `NV_ERR_NOT_SUPPORTED` for that sentinel;
`tmrapiGetRegBaseOffsetAndSize_IMPL` propagates it. This can fail after a successful
allocation without another GSP RPC. No Windows stack trace yet confirms that
this is where probe C fails. Do not advertise the timer range until its reads
are actually served through an audited mapping.

## Probe D: genuine read-only timer mapping

**The hypothesis above did not resolve the observed failure.** Product revision
`0e92e959952873d8084f9313353ea38a9d1c4109`, built with QEMU 10.2.4, ran a fresh
overlay with both `--pool-probe` and `--timer-map`. Realize reports the host-derived
timer base and genuine read-only 4096-byte backing, with matching compiled host
and guest layouts. The prior native probe passed three map/read/drop cycles as
UID/GID 65534, no groups/capabilities, no-new-privileges; its evidence and source
audit are in `traces/windows_timer_20261005/`.

Windows still reports Code 43, NVIDIA's `DEVPKEY_Device_ProblemStatus` is zero,
and nvidia-smi exits 9. **All 208 traced RPC lines exactly match probe C**, with
210 total serviced messages and no GPU channel births. A real timer page fixes
an unsupported feature but has not advanced this Windows initialization. This
does not establish whether Windows reached or used that mapping before failing.
The separate Basic Display adapter reports Code 10 / `0xc01e0438`
(`STATUS_GRAPHICS_NOT_POST_DEVICE_DRIVER` in the Windows headers); do not confuse
that with the NVIDIA device's error.

The guest shut down cleanly and QEMU exited zero. Files in `probe-d/` include the
full command, QEMU/serial logs, normal status and additional device properties.
The latter script exits 1 when no matching event-log entries are found; device
property queries themselves succeeded. No missing-event output establishes an
absence of an internal NVIDIA error.

## Probe E: current master plus private Translated spaces

Unlike A–D, this binary comes from integration branch
`codex/p1p2-integration-2026-10-05`, product revision
`e71e4a8b21671d14b89c25602e78b687e8463333`, runner revision `0880e340`.
It includes current master `906a76a4`, the full P1/P2 branch, the Windows pool
experiment and timer mapping. All three opt-ins are enabled. QEMU 10.2.4 built
successfully; the initial build needed `traces/driver_matrix` added to the bench's
sparse checkout. No product guard was bypassed.

The fresh overlay still reports Code 43 / nvidia-smi exit 9. All 208 traced RPC
lines match C and D exactly (`probe-e-initial/rpc-comparison.json`), with 210
serviced messages and zero GPU channel births. The logs report
`tspace[built=yes]`; this does **not** test channel isolation because no guest
GPU channel is born. The exact-revision hardware gates and adversarial isolation
probes remain separate requirements. `probe-e-initial/` is a snapshot before any
debug-tool installation or adapter restart; QEMU was still running.

### Kernel diagnostics on E's disposable overlay

Microsoft-signed DebugView CLI 5.02 captured a bounded 90-second kernel stream
while `pnputil /restart-device` restarted the sole NVIDIA adapter. Both commands
returned zero. This produced another GSP initialization sequence and retained
Code 43. `probe-e-debugview/` contains the capture, adapter status, tool identity
and exit records. `kernel.csv` is the tool's tab-separated log despite the suffix;
`capture.stdout` contains CSV output. No NVIDIA driver binary or product policy
was changed.

The stream records NVIDIA startup and WER creating a WATCHDOG live dump. WER's
classification is `0x1b0`, parameter 1 `2`, parameter 2 `0xffffffffc000009a`.
Microsoft documents this as **video miniport StartDevice failure**, with the
second parameter holding NTSTATUS; it is a diagnostic live dump, not a system
bugcheck ([reference](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x1b0--video-miniport-failed-livedump)).
`0xc000009a` is `STATUS_INSUFFICIENT_RESOURCES`. This identifies the reported
failure class; it does **not** identify the failing allocation, prove physical
RAM exhaustion, or establish which RPC response caused it. The captured NVIDIA
messages contain no more precise RM error.

Both the initial-start dump and restart dump are preserved privately on the
controller; only hashes/sizes and selected WER classification fields are public.
Raw dumps and the machine's complete WER inventory are deliberately excluded.
