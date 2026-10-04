# Windows load investigation — 2026-10-04

Target: disposable Windows 11 build 26100 nested KVM guest, without an NVIDIA
GPU. Test signing is active; Secure Boot, VBS and HVCI are inactive. Driver
installation and all probes use the controller's pinned SSH connection. These
checks do not establish NVIDIA/GSP attachment or capture.

## Certificate-store initialization

The clean image had no `HKLM\SOFTWARE\Microsoft\SystemCertificates\TrustedPublisher`
store. `Import-Certificate` returned `E_ACCESSDENIED` at TrustedPublisher from
both elevated SSH and a SYSTEM task; Root import succeeded. This refutes an
SSH-token-only explanation. Installer commit `c72cdb67` uses
`X509Store.Open(ReadWrite)` to create/open the named machine stores and adds the
exported public certificate. Direct SSH then successfully signed and verified
the driver. No certificate-store ACL or Windows integrity policy was changed.

Microsoft documents store creation when `OpenExistingOnly` is omitted:
[X509Store.Open](https://learn.microsoft.com/en-us/dotnet/api/system.security.cryptography.x509certificates.x509store.open?view=netframework-4.8.1).

## Image-loader probes

`sc start` returns Win32 error 487. The exact `NtLoadDriver` result is
`0xC0000018` (`STATUS_CONFLICTING_ADDRESSES`). Diagnostic source `5c1a85da`
writes its own service's `InitDiagnostic` value upon entering `DriverEntry`.
Before each attempt the value is cleared and service-key write access checked.
The missing marker places the observed failure before `DriverEntry`.

All variants below retain GS, NX, ASLR, test signatures and integrity checks.
The source observer logic is identical. Binaries were signed on the target;
SHA256 values identify the trusted **unsigned** build inputs.

| Variant | SHA256 | Outcome |
| --- | --- | --- |
| Original normal build | `9767a086b5016c6f8e90a95876714b406b8efc840f33541886451c7d89534c17` | Service error 487; signing verifies |
| Baseline with entry diagnostics | `91074afae194f8058ae362b6f683f76aa8dca0ed7286096de9435620a54afd7f` | `C0000018`, entry marker absent |
| Baseline plus `/section:.retplne,R` | `849c2df70ac4ad684f3c0fe2a0890e119d3831b01756e3550ec60a0065a2b591` | `C0000018`, entry marker absent |
| Same objects/flags, WDK 26100.1 libraries | `64a7eb7803ff6123850ae43024eb1d05da5f6b97d9c013c426b8fad5b5957fe6` | `C0000018`, entry marker absent |
| Baseline plus `/version:10.0` | `a9e2bc36e977211b92113da39830cb2910f3e8346e9d91b7a5825c4120c71481` | `C0000018`, entry marker absent |
| Minimal GS entry/unload probe | `1e66716d84bc975b70f210fb74396c4f1510dd5197573022a78e7006fe043171` | `C0000018`, entry marker absent |
| Baseline plus `/tsaware:no` | `9c34736e1767fbdbfc0e5add5d9f1592eaf86ac2fddead93bda91630e3d3d80f` | Bugcheck `1A/101B`, entry marker absent; recovered by automatic reboot |
| Baseline plus `/tsaware:no /section:.retplne,R` | `9191f961588bc0b17c70dc3cb8cce03ab75dba5fcc796fda8c4748c9287dc021` | Load success, entry stage 5, API tests pass, unload success |

The `/tsaware:no` attempt passed the previous immediate rejection but caused
`MEMORY_MANAGEMENT` with arguments `101b`, `fffff8045fd18000`,
`9f8b484aff500400`, `ffffffffc0000005`. The `.retplne` section is at RVA
`0x8000` and has no memory permissions in that image. A fault on that section
is a working hypothesis, not yet established: the image base/stack has not
been recovered from the target dump. The earlier readable-section attempt
still had TSAWARE set, so its immediate rejection does not rule out a second
independent section-permissions problem. The combined variant subsequently
loaded with `NtLoadDriver=0` and entry stage 5/status 0. Its API tests passed
with zero failures, covering buffer/IOCTL validation, exclusive opening,
restricted-token denial and worker stop. `NtUnloadDriver` returned zero.
Stats before the test showed 928,124,899,328 scanned bytes across 108 passes,
zero read failures and no attached tables on this GPU-less VM. Do not use the
TSAWARE-only image for capture.

Actual kernel `SystemCodeIntegrityInformation` reports options `0x00080207`,
including the active test-signing bit `0x2`; this is stronger evidence than
the BCD setting alone.

The image has DIR64 relocations; all recorded target addresses lie inside its
sections. Characteristics `0x22` (no DLL bit) and preferred base `0x140000000`
also occur in locally available working NetKVM and paguro drivers. These fields
alone therefore do not explain the failure. The official WDK normal WDM props
do not explicitly set `LinkDLL`; its export-driver props do. WDK common props
set image version 10.0, while the initial cross-link command left it at 0.0.

LLD 19 and 21 do not process the WDK `.retplne` compiler metadata. The baseline
contains that section with no memory permissions, but making it readable did
not resolve the loader failure. WDK 26100.1 shrinks the load-config directory
from `0x148` to `0x140`; that comparison also failed before entry.

## Reproducing the unsigned variants

First build the diagnostic objects using the ordinary pinned kit:

```sh
KFGT_CACHE=/data/windows-gsp-build \
KFGT_OUTPUT=/data/nouveau-trace-research/gsp-load-diag \
KFGT_INIT_DIAGNOSTICS=1 tools/windows-gsp-trace/build-linux.sh
```

The baseline relink command, with task-specific paths matching this run, is:

```sh
lld-link-19 /driver /subsystem:native,10.0 /osversion:10.0 \
  /entry:GsDriverEntry /machine:x64 /nodefaultlib /dynamicbase /nxcompat \
  /integritycheck /release /Brepro \
  /out:/data/nouveau-trace-research/gsp-load-diag/gsptrace.sys \
  /data/nouveau-trace-research/gsp-load-diag/gsptrace.obj \
  /data/nouveau-trace-research/gsp-load-diag/queue.obj \
  /libpath:/data/windows-gsp-build/wdk/c/Lib/10.0.28000.0/km/x64 \
  ntoskrnl.lib hal.lib wdmsec.lib BufferOverflowK.lib
```

Each comparison changes only the table's stated argument or library path and
the `/out:` filename. The older libraries come from the official NuGet package
`microsoft.windows.wdk.x64/10.0.26100.1`, archive SHA256
`247b2919ae451f65ba5f1cd51c7c39730fb0fc383d607f3e8ab317fddc8a8239`.
Their extracted path is
`/data/windows-gsp-build/wdk-26100.1/c/Lib/10.0.26100.0/km/x64`.

Controller text evidence for this run is retained under
`/data/vast-windows-runtime/54159260/`, including `nested-driver-smoke-3.stdout`,
`ntload-probe.stdout`, `load-diag-2.stdout`, `load-diag-readable.stdout`, and
`load-diag-wdk26100.stdout`. No target executable was copied back to the
controller, and no SSH private key was copied to the rental.

## Minimal runtime/loader probe

`tests/load_probe.c` is a diagnostic-only image that writes the same entry
marker and has an unload callback. It creates no device, worker or observer.
It keeps `/GS`, `GsDriverEntry`, NX, ASLR and the same WDK 28000
`BufferOverflowK.lib`, with only `ntoskrnl.lib` otherwise required. Build it
with `tests/build-load-probe.sh` after populating the standard cache. Initial
unsigned SHA256 is
`1e66716d84bc975b70f210fb74396c4f1510dd5197573022a78e7006fe043171`;
the target returned the same `C0000018` without the entry marker. This narrows
the failure to the image/toolchain/runtime rather than observer logic, HAL or
the secure-device library. Never use this image for capture.

## Independent Microsoft compiler/linker comparison

`.github/workflows/windows-gsp-build.yml` runs `build-msvc-ci.ps1` on an
official Windows runner with its installed Visual C++ toolchain and the same
SHA256-pinned Microsoft SDK/WDK NuGet archives as the Linux build. It publishes
unsigned normal/diagnostic observers and the minimal GS probe, plus tool
versions, exact commands, source revision and artifact hashes. It does not
sign or load a driver, and a passing CI build is not a Windows runtime result.

Run [37217644450](https://github.com/reindertpelsma/kayfabe/actions/runs/37217644450)
successfully built source `351d5b7f` with MSVC compiler `19.44.35229.0` and
linker `14.44.35229.0`, keeping `/W4 /WX`. The initial attempt exposed two
MSVC compatibility issues: `/kernel` reserves/defines `_KERNEL_MODE` itself,
and the WDK kernel CRT lacks `stdint.h`. The shared protocol header now uses
the compiler's fixed-width integers in kernel translation units instead of
mixing the user-mode CRT headers into them. Linux build output stayed
byte-for-byte identical after that source change; the portable parser and
11 Python tests passed.

The successful MSVC diagnostic observer SHA256 is
`751f87043dcc167dc132fc0707a312fce3b3d7a24167adc9c2a4f2d2e695176e`,
normal observer
`c32adf94bde55c19caa493b9ccc460bd39926900acfc4ca93297a6787bb1c3ca`,
and minimal GS probe
`af315af9f07235aa9ba547a06723d88b3f168ede28c6ed5084b8af7ef21a1293`.
These native images have TSAWARE clear, omit the compiler-only `.retplne`
section, and mark the ordinary code/data sections nonpageable. Their target
load/API results were then tested separately: the normal observer and MSVC
user tools passed ordinary SCM installation, signing and load, all API tests
(zero failures), service restart/status, and stop on the same Windows VM.
Evidence is `msvc-smoke.stdout` in the controller runtime directory. The final
service was left demand-start and stopped; no NVIDIA GPU was attached for this
test. This establishes the MSVC binary's basic Windows operation, not GSP
queue discovery or cross-GPU compatibility.

## Revised Linux linker defaults and limits

The revised scripts set `/tsaware:no`, `.retplne` read/nonpageable attributes,
and explicit nonpageable `.text`, `.rdata`, `.data`, and `.pdata` attributes to
match the corresponding Microsoft linker behavior. NX, ASLR, GS and integrity
checks remain enabled. `tests/check_driver_pe.py` checks these image properties
plus entry/relocation bounds and the GS cookie before a successful build is
reported. It rejects the original image and accepts the three MSVC CI images.

The revised normal Linux observer SHA256 is
`89a04d6bba83db0fe077d90df121826f7b3eb66d213278c4a5381c6f73b4edad`;
the revised minimal probe is
`5233d314242240de6c32f891f69ef08e1bed860cf263b877431d3249cd5c8cc1`.
The added nonpageable attributes mean these are not byte-identical to the
successful combined diagnostic probe. Their runtime parity remains untested.
Use the separately smoke-tested Microsoft-linker artifact for the next
capture rehearsal.

## First RTX 4070 rehearsal and text export

The same trusted MSVC normal observer and user tools also passed install,
API tests, restart/status and stop in a Windows guest with an RTX 4070 passed
through directly by VFIO. NVIDIA's display driver had not yet been installed.

The first installation rehearsal stopped before NVIDIA ran: the native setup
helper successfully extracted its package, but its `Start-Process` wrapper
lost the child's exit code and refused to continue. The collector wrapper
likewise recorded a null exit code. That value remains **unknown**, rather
than being reclassified as success. The observer's final stats independently
showed 1,065,811,304,448 scanned bytes over 124 passes, zero attached tables,
zero read failures, zero records/drops, and an empty FIFO after drain.

`export-trace.ps1` successfully produced text JSONL on the Windows guest.
Offline `decode.py --require-query-pair` verified its complete source hash and
header, then returned 4 because there was no query pair. The original capture
was exactly 64 bytes, SHA256
`9695e840707d9f34fee30eba28c15a9fd4be0d37bc2efd5680dc4cc84b44261c`.
This is a valid empty observation file, consistent with NVIDIA never starting;
it does not test GSP queue discovery. Only text logs returned to the controller.
Evidence is under
`/data/vast-windows-runtime/claude-20261004/first-capture-text/`, with the
separate failed-wrapper state in `drain-first4070-recovered.stdout`.

The replacement native process wrapper owns a `Diagnostics.Process` from
`Start()` through `ExitCode`, with a timeout and explicit null-code rejection.
Its actual Windows PowerShell 5.1.26100.1591 smoke observed exit codes 0/1/7,
20 fast successful children, rejection of an unexpected nonzero code, and
bounded tree termination after a one-second timeout. This helper belongs to
the separate installer repository; the recorder driver and collector binaries
were unchanged by that wrapper fix.

## GSP mode and registry timing before a controlled reboot

A subsequent RTX 4070 capture covered the actual 580.88 NVIDIA installation.
Its numeric collector exit was 4 and the exported file remained header-only
(SHA256 `8d3dfb99989ef730a83ea7338a9cf2da0c4cb7d1369e38241ece095d63e49308`).
Offline JSONL decoding verified the source hash and framing. The driver was
running, PnP reported Started/Problem 0, and `nvidia-smi -q` succeeded, but
reported **GSP Firmware Version: N/A**. Both current service and display-class
`EnableGpuFirmware` values were 1. These registry values alone do not prove
that the first driver initialization consumed the policy: the class value was
written only after `setup.exe` had already started the device.

The trusted controller independently downloaded the pinned official 580.88
installer, verified its existing installer-manifest size/hash, and extracted
`nvlddmkm.sys`. Its SHA256
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`
matches the target's text report. No executable was copied back from the
rental. Offline inspection provides a concrete registry-path check:

- The UTF-16 `EnableGpuFirmware` string is at RVA `0xd496d0`; its direct
  code reference at `0xe6531` leads to a value read through helper `0xe5b00`.
- On Windows version 10/build >=25850, that helper uses the device-aware
  path. Helper `0xe56d0` calls imported `IoOpenDeviceRegistryKey` with flag 2,
  which WDK `wdm.h` names `PLUGPLAY_REGKEY_DRIVER`: the device software key.
- The value, or default `0x12` when unavailable, feeds the firmware-policy
  function at RVA `0x141360`. Its mode-1 branch requests firmware when the
  device is considered capable. This is static evidence, not proof of live
  firmware execution or a promise that all Windows/GPU combinations support it.

The published OGKM `nv-firmware-registry.h` also explicitly identifies the
key as shared by Windows and Unix. Therefore the next controlled test keeps
the existing class policy and reloads/reboots with the observer running early;
there is no evidence yet that a different key name or queue layout is needed.
The current collector exposes the preexisting ABI's `candidates` counter to
distinguish no physical self-reference candidates from later validation
failure. This collector-only change does not alter the kernel binary.

The controlled reboot subsequently activated GSP firmware 580.65.05 and the
observer attached successfully. The [preserved full text export and analysis](evidence/2026-10-04-rtx4070-580.88/README.md)
validate 4,535 request/reply records. No target query was observed; the first
retained queue sequences are already 2707/2710, so the early initialization
prefix is unknown. This proves passive Windows GSP capture on that target,
not recovery of the pool-sizing algorithm or cross-GPU compatibility.
