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
target result is pending. Never use this image for capture.
