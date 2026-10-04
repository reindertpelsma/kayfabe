# Windows GSP observer

**STATUS: RESEARCH, 2026-10-04.** A diagnostic observer loaded on Windows 11
build 26100, passed the Windows API/negative-access tests, and unloaded cleanly
in a nested VM without an NVIDIA GPU. MSVC and Linux builds pass compilation
and static PE checks. The MSVC normal observer and user tools also passed
installation, API tests, restart/status and stop on that VM. Runtime parity of
the revised Linux default binary remains untested. The tested MSVC observer
also captured 4,535 valid Windows GSP records on an RTX 4070 with driver 580.88
and firmware 580.65.05. The early prefix was missed and no pool-size query was
observed; [preserved evidence](evidence/2026-10-04-rtx4070-580.88/README.md) and
[LOAD_TESTS.md](LOAD_TESTS.md) document the exact hashes and limits.

The purpose is to collect actual request/reply bytes for controls such as
`GR_GFX_POOL_QUERY_SIZE` (`0x2080121f`), including the successful reply, so Kayfabe
can investigate a rule based on capabilities rather than a per-GPU constant.

## Optional headless graphics trigger

`tests/d3d11_probe.c` enumerates DXGI adapters and refuses unless exactly one
NVIDIA hardware adapter exists. It creates a D3D11 device and immediate context
on that explicit adapter, clears/copies a small texture, waits for a GPU event,
and verifies a pixel readback four times. It uses no window or swap chain and
has no software-renderer fallback. `--list` only enumerates adapters. Output is
JSONL with adapter identity, HRESULTs and pixel results; exit zero requires all
four readbacks and the final device-health check to succeed.

Build with the scoped MSVC workflow, or on a trusted Linux controller:

```sh
KFGT_PROBE_OUTPUT=/tmp/kayfabe-d3d11-probe bash tools/windows-gsp-trace/tests/build-d3d11-probe.sh
```

Start the observer and collector before invoking `d3d11_probe.exe` without
arguments. Use an **external 60-second process timeout**: each event wait is
bounded to ten seconds, but calls into the graphics driver can themselves block.
Preserve stdout, stderr and the numeric process exit before stopping/draining
the collector. Compilation is not a hardware runtime test, and successful
rendering does not imply the target control was called. A query executed only
during early driver initialization can still require a separate initialization
capture. This probe neither restarts PnP devices nor changes firmware policy.

The explicit-adapter call follows Microsoft's
[D3D11CreateDevice contract](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-d3d11createdevice)
with `D3D_DRIVER_TYPE_UNKNOWN`; event completion is checked through
[GetData](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-getdata).

## What the driver does

The demand-loaded x64 Windows driver scans ordinary physical RAM for the
published GSP shared queue allocation. Its first physical page-table entry points
back to the table itself. The next page describes the CPU-to-GSP command queue;
the status queue follows the command queue in that table. The driver validates
both headers, every page against the OS RAM ranges, alignment, uniqueness, and
size bounds before attaching. It supports eight allocations, queue format 0,
4096-byte entries, and a **single page of physical page-table entries**. The
default 256 KiB command + 256 KiB status queues fit this profile.

A worker copies each queue twice into private nonpaged memory and accepts only
identical snapshots with unchanged mappings. The decoder checks message lengths,
RPC header version/signature, plaintext authentication fields and the transport
XOR checksum. Only messages at or before the published queue tail are eligible.
It retains available history, sorts by transport sequence, suppresses duplicates
and marks observed sequence gaps. A 32 MiB output FIFO drops incoming records if
full and counts those drops. The driver reads with `MmCopyMemory`; it never maps
MMIO, writes GPU memory, modifies queue indices, installs hooks or patches
`nvlddmkm.sys`.

The control device is exclusive and grants access only to SYSTEM and
Administrators. IOCTLs are limited to stats, draining records and stopping the
worker. There is no caller-selected physical-address reader or writer. Memory,
message sizes, table count and each scan iteration are bounded. Unload joins the
worker before freeing memory.

## Limits that matter for interpretation

- This is **passive sampling**, not a lossless interception hook. Every file and
  record says so. A first observation has an unknown prefix even if its first
  transport sequence is zero. Full ring laps between polls, discovery delay,
  unstable snapshots, queue reuse and FIFO overflow can all lose messages.
- Timestamps are when bytes were observed. Retained history can predate driver
  loading or collector start. The ring can preserve useful initialization
  requests/replies, but no particular early query is guaranteed to remain.
- Discovery scans all RAM. Its speed and impact must be measured on the target;
  run the recorder before enabling/installing NVIDIA. Polling copies shared
  memory and consumes CPU/memory bandwidth. The worker yields after 64 scan
  chunks; the requested 1 ms wait is subject to Windows timer scheduling.
- The signature identifies a published queue **shape**, not cryptographic
  ownership by NVIDIA. Use a dedicated single-GPU research VM and save host
  metadata. Multiple queue allocations are distinguished by physical table
  address, not automatically assigned to PCI device IDs. Stale allocations can
  persist after the NVIDIA driver stops; a new allocation requires rediscovery.
- DMA/IOMMU addresses that differ from CPU physical addresses prevent this
  discovery method from working. Larger page tables, a different private
  transport, and confidential-compute encrypted traffic are unsupported. It
  refuses unknown layouts rather than guessing. No private Windows symbols are
  required when this published layout applies.
- A successfully parsed file is not evidence of a complete trace. One successful
  query pair on one GPU/driver does not establish a cross-family sizing rule.

## Build on Linux; no compiler needed on the Windows target

Install `clang-cl`, `lld-link-19`, `x86_64-w64-mingw32-gcc`, Python 3 and curl, then:

```sh
./tools/windows-gsp-trace/test.sh
./tools/windows-gsp-trace/build-linux.sh
```

`KFGT_CACHE` selects the download cache and `KFGT_OUTPUT` the output directory.
`CLANG_CL` / `LLD_LINK` override compiler/linker paths. The script verifies pinned
SHA256 hashes of Microsoft's SDK and WDK NuGet packages before extraction. It
builds an unsigned native PE driver with GS, NX and ASLR enabled, the collector,
and a Windows API test executable. `build/signing/` contains the original
Microsoft SDK `signtool.exe`, its manifests and dependencies (about 5.6 MiB).
This avoids installing Visual Studio, the SDK or WDK on the target. Downloaded
Microsoft binaries remain outside version control; retain their licence terms.

The Linux linker explicitly clears TSAWARE, gives the WDK metadata section read
permission, and marks ordinary kernel code/data nonpageable. `driver-pe.json`
records static checks for those flags, relocation/entry bounds and the GS
cookie. These checks prevent the malformed images found during bring-up; they
do not replace a target load/API/unload test. Prefer the MSVC artifact for the
first capture until runtime parity of the revised Linux build is established.

Transfer this directory's source/scripts and `build/` to the disposable Windows
VM. Alternatively, `build.cmd` builds with MSVC in a VS x64 Native Tools prompt
with the SDK and WDK installed; this native build path has not been run here.

For an independent compiler/linker comparison, the scoped GitHub Actions
workflow `windows-gsp-build.yml` runs `build-msvc-ci.ps1` on a Windows runner
with its installed MSVC toolchain and pinned SDK/WDK NuGet packages. Its
unsigned artifact includes the normal observer, diagnostic observer, minimal
loader probe, user tools, source revision, hashes, commands and compiler
versions. This does not load or sign the binaries. Current target load
investigation results are recorded in [LOAD_TESTS.md](LOAD_TESTS.md).

## Prepare and test the Windows VM before replacing any boot disk

Use Windows 10 2004+ or Windows 11 x64. Keep a separate shell for the collector.
The driver is deliberately test-signed for a disposable research VM. The nested
Windows image should have Secure Boot disabled and test signing enabled before
its final boot rehearsal; a normal production signed-driver configuration will
reject the test driver.

In elevated PowerShell:

```powershell
Set-Location C:\kayfabe\tools\windows-gsp-trace
.\install.ps1 -Action EnableTestSigning
# Reboot Windows explicitly, and verify SSH still works.
.\install.ps1 -Action Install
.\build\windows_api_test.exe
if ($LASTEXITCODE) { throw 'Driver API tests failed' }
sc.exe stop KayfabeGspTrace
sc.exe start KayfabeGspTrace
```

`Install` finds `build/signing/signtool.exe` automatically, creates a throwaway
local code-signing certificate, imports its public certificate into the target's
Root/TrustedPublisher stores, signs the copied driver and starts it. An installed
SDK's signtool or `-SignToolPath` can be used instead. It neither reboots nor
automatically disables Secure Boot. Preserve the reported certificate thumbprint
if you intend to clean up instead of discarding the VM.

`windows_api_test.exe` checks stats, worker progress, invalid IOCTLs, input and
output lengths, exclusive opening, rejection of child paths, denial to a
restricted token, and synchronous worker stop. It needs no NVIDIA GPU. **It
stops the recorder**; restart the service afterward. Run these tests before any
valuable capture. Windows runtime results must be recorded separately; the
existence of this executable is not a passing result.

## Optional staging through the Vast Windows preparation VM

For a driver load failure, retain the exact `NtLoadDriver` status as well as
the Win32 service error. An optional diagnostic build separates loader failure
from `DriverEntry` failure:

```sh
KFGT_INIT_DIAGNOSTICS=1 KFGT_OUTPUT=/tmp/gsp-load-diag ./build-linux.sh
```

This variant writes an eight-byte `InitDiagnostic` value in its own service
registry key: a little-endian stage DWORD and NTSTATUS DWORD. Stages are
0 (entered), 1 (device creation), 2 (allocations), 3 (worker creation),
4 (symbolic link), and 5 (ready). Clear the previous value before a controlled
load attempt; an absent value is useful evidence only if the service key is
writable. Normal builds contain no diagnostic registry writes. Sign the
diagnostic driver on the target before loading it, and retain its file hash
with the result.

`stage-qga.py` copies trusted controller-built files into
`C:\ProgramData\KayfabeGsp` during the installer's final cold-boot SSH rehearsal
hold. It requires the native GPU setup task to exist and remain deferred. Build
and create the bundle on the trusted controller, then copy the bundle and stager
to the Linux rental:

```sh
python3 tools/windows-gsp-trace/stage-qga.py bundle /tmp/gsp-stage.zip
# Save the printed SHA256; copy this public build/source bundle to the rental.
python3 stage-qga.py stage /root/gsp-stage.zip --sha256 PRINTED_SHA256
```

To package a trusted downloaded MSVC CI artifact, add
`--build-directory /path/to/extracted/artifact` to `bundle`. The packer verifies
the executable hashes against `build-info.json`, includes that provenance and
the build log, and keeps the build source revision distinct from the bundled
scripts' revision. It reuses the pinned Microsoft signing tools in local
`build/signing/` if the CI artifact has none. Never use binaries copied back
from an untrusted rental as build inputs.

The rental-side command defaults to `/var/lib/vast-windows/qga.sock` and the
installed `/root/vast-windows/prepare/prepare.py` guest-agent helper. It requires
`login-test.json` and refuses a released `login-test-complete` marker. The target
is the installer's English Windows image. It verifies the archive and every
file in Windows, smoke-tests bundled signtool and collector execution, and sets
test signing for **the next boot**. It also records Secure Boot and Device Guard
state without changing memory-integrity/HVCI settings. It writes `gsp-stage-result.json` on Linux and
`stage-result.json` in the guest. It does not install/start the recorder, start
NVIDIA, reboot, release the preparation hold, or arm the disk flasher. The native
boot still needs `install.ps1 -Action Install`, API tests, recorder restart and
capture before releasing NVIDIA's separate deferred-install hold.

## Capture

```powershell
.\metadata.ps1 -OutputPath C:\traces\host.json
.\build\gsptrace.exe C:\traces\gsp.kgwt 1800 --stop-file C:\traces\gsp.stop
# While the collector runs, install/enable NVIDIA with GSP enabled in another shell.
```

For a rental that permits returning text logs only, stop/drain the collector,
then export structured JSONL on Windows without installing Python:

```powershell
.\export-trace.ps1 -Path C:\traces\gsp.kgwt -OutputPath C:\traces\gsp.jsonl
```

Return only `gsp.jsonl`, metadata JSON and ordinary logs to the controller;
keep the binary capture and any crash dump on the rental. The exporter writes
bounded record metadata and payload hex, hashes the complete original capture,
includes the statistics sidecar when present, and atomically publishes a new
UTF-8 text file. It refuses live writers, malformed/truncated framing,
existing output paths and bounds violations. Defaults are 64 MiB of input and
100,000 records; explicit `-MaxInputMiB`/`-MaxRecords` may raise those bounds.
The footer describes a complete **file export**, never a complete GSP capture.

```sh
python3 tools/windows-gsp-trace/decode.py gsp.jsonl --require-query-pair
```

The decoder verifies the footer/hash, rebuilds bounded records in memory and
applies the same checksum, ABI, sequence and request/reply validation as for a
local binary capture. No executable or memory dump is returned or run. Its
decoded-input limit defaults to 64 MiB (`--max-input-mib` to change explicitly).

For a large capture, prioritize the query evidence with
`-GfxPoolOnly -MaxInputMiB 512`. This streams the source while retaining only
RM_CONTROL `0x2080121f` requests/replies in the text log. It reports omitted
record counts and separate hashes for the original file and the selected
record stream. The decoder can verify the selected stream; the full-source
hash is provenance only because omitted bytes are absent. Sequence gaps in
this mode can include deliberately omitted traffic. Keep the original capture
on the rental if further controls need to be investigated. Full export remains
available with explicit bounds, e.g. `-MaxInputMiB 512` and the decoder's
matching `--max-input-mib 512`.

Create `C:\traces` beforehand. The collector stops the worker, drains the entire
FIFO and writes `gsp.kgwt.stats.json`. It flushes the C output buffer after each
read batch and checks trace/statistics write, flush and close failures. A sudden
process termination or reboot can still leave a partial record; drain and close
the collector before a research reboot. Ctrl+C follows the same drain path. Exit 4
means no validated messages were recorded. Restart the service before another
run. For a detached SSH capture, create the stop marker from a second session:

```powershell
New-Item -ItemType File C:\traces\gsp.stop
```

The collector checks for it every 250 ms while retaining the duration bound,
stops the kernel worker and drains/closes the output normally. The marker must
be absent at startup, name a file rather than a directory, and differ from the
trace output. Existing markers are refused rather than silently removed. Keep
it in a directory writable only by the authorized collector user/Administrators;
remove it explicitly before the next capture. A marker inspection error also
stops/drains but exits with failure. This is preferable to killing the detached
collector process. Save the metadata, NVIDIA firmware/driver version (`nvidia-smi -q`), GPU PCI
ID/revision, driver binary hash, code revision and application/device-start result
alongside every recording. Registry settings alone do not prove that GSP ran.

On Linux or Windows with Python:

```sh
python3 tools/windows-gsp-trace/decode.py gsp.kgwt --require-query-pair > query.json
```

The decoder rejects malformed/truncated data. Large RM_CONTROL messages can use
continuation records: those raw records are retained, and declared versus observed
parameter sizes are reported. An incomplete control is never decoded as a complete
40-byte query reply. It prints independent query
observations and pairs only an unambiguous matching RPC sequence, allocation,
client/object and `maxSlots`. Exit 4 means no successful query pair was found.
The fields are `maxSlots`, `slotStride`, `ctrlStructSize`, `ctrlStructAlign`,
`poolSize` and `poolAlign`. Reply success requires both RPC and RM-control status
zero. There is no automatic replay or generated per-die response table.

Remove the service with `install.ps1 -Action Remove`. If retaining the VM, remove
the test certificate from My/Root/TrustedPublisher by its recorded thumbprint and
explicitly turn test signing off with `bcdedit /set '{current}' testsigning off`
followed by a reboot. The script leaves those host settings for explicit cleanup.

## Evidence and ABI sources

- OGKM `580.159.04`, commit `b81d58ee0224d1d290bef1c080592b619e184042`:
  `src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c`,
  `src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h`,
  `src/common/shared/msgq/inc/msgq/msgq_priv.h`,
  `src/nvidia/generated/g_rpc-message-header.h`, `g_rpc-structures.h`,
  and `src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gr.h`.
- These offsets are derived from the published wire ABI; NVIDIA implementation
  code is not copied into this driver. The existing Linux in-driver recorder is
  in `scripts/rpctrace/`; its lossless hook semantics do not apply here.
- [Microsoft MmCopyMemory](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/nf-ntddk-mmcopymemory)
  defines the safe physical-RAM read API and its limits.
- [Microsoft test signing](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/the-testsigning-boot-configuration-option)
  requires a signed test binary even when HVCI is enabled; test signing does not
  justify preemptively disabling memory integrity.
- [Microsoft device security](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/sddl-for-device-objects)
  defines the SYSTEM/Administrators DACL used by `IoCreateDeviceSecure`.

Verified locally: clang-cl 21 + lld 19 against official Microsoft
`10.0.28000.2526` SDK/WDK, MinGW collector/API-test builds, GCC and Clang
ASan/UBSan queue tests, Python decoder rejection/pairing tests, and PowerShell
syntax parsing. A reconstructed-ring replay of all 1,076 payloads in the archived
Linux GA106 boot capture also passes the actual observer core and decoder, including
wrapping and continuation records. Request checksums are reconstructed because the
Linux TX hook runs before the checksum is computed. Those replay checks do
not establish live GPU attachment or Windows query capture. Separate Windows
load/API/unload evidence is recorded in [LOAD_TESTS.md](LOAD_TESTS.md).
