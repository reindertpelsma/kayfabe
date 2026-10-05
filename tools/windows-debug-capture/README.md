# Windows kernel diagnostics for the Kayfabe boot failure

**STATUS: RESEARCH, 2026-10-05.** The pinned tool and wrapper passed Windows preparation and a
90-second kernel capture on the Kayfabe Windows E overlay. Adapter restart returned success;
the capture contains 21 kernel messages including NVIDIA initialization and creation of a
WATCHDOG live dump. The signed KD wrapper also completed semantic dump analysis. Code 43
remains. This supplements GSP traffic with diagnostics the Windows driver actually emits.
It does not change Kayfabe policy or patch `nvlddmkm.sys`.

Use a fresh **diagnostic overlay**, preserving the immutable Windows fixture. The wrapper only
supports elevated x64 Windows and installs Microsoft DebugView CLI 5.02. The official package
contains a native CLI, bounded capture, and boot logging; those features are documented by
[Microsoft](https://learn.microsoft.com/en-us/sysinternals/downloads/debugview). The actual ZIP's
CLI help/parser strings contain the parameters used below. Before any execution, the Windows
wrapper additionally requires a valid Microsoft Corporation Authenticode signature and the
exact executable hash. Help/version are then executed and checked on the target.

| Artifact | SHA256 |
|---|---|
| `https://download.sysinternals.com/files/DebugView.zip` | `a8454253756af10667b82faf2323de536f0b7084d732acba63803df01ce4c316` |
| x64 `dbgviewcli64.exe` (488800 bytes) | `7954a8bbeb1f650bb5d2b8c4c6b642fd4585319d2092433ca841c7d672a4634b` |

The executable's embedded certificate names Microsoft Corporation, issued by Microsoft Code
Signing PCA 2024. Reading an embedded certificate is **not** signature validation; target
`Get-AuthenticodeSignature` must report `Valid`. An upstream package update fails the hash check
until separately inspected. No Microsoft executable or signing material belongs in Git.

## First experiment: runtime capture plus adapter restart

Prepare the trusted bundle on the controller:

```sh
bash tools/windows-debug-capture/fetch.sh /tmp/kf-debugview-bundle
```

Transfer that bundle to a disposable Windows overlay. Then run these commands through the
existing authenticated administrative SSH session (PowerShell 5.1 is sufficient):

```powershell
# Parse before running when first deploying a changed script.
$tokens=$null; $errors=$null
[System.Management.Automation.Language.Parser]::ParseFile(
  'C:\ProgramData\KayfabeDebugViewStage\debugview.ps1', [ref]$tokens, [ref]$errors) | Out-Null
if ($errors.Count) { $errors | Format-List *; throw 'PowerShell parse failed' }
& 'C:\ProgramData\KayfabeDebugViewStage\debugview.ps1' -Action Prepare
& 'C:\ProgramData\KayfabeDebugViewStage\debugview.ps1' -Action Capture -Seconds 90 -RestartAdapter
```

The capture disables Win32 collection, enables verbose kernel collection, and has three bounds:
90 seconds, 50000 emitted lines, and a 16 MiB wrapping log (20000 history lines). It preserves
stdout/stderr, tool identity, command exit, adapter state and the restart result. In the tested
5.02 CLI, `--format csv` controls the record stream in `capture.stdout`; the separate `--log`
file `kernel.csv` actually contains tab-separated records despite its suffix. Preserve both.
The adapter restart selects exactly one present PCI NVIDIA display adapter; no restart occurs
if capture never reports ready. A pre-existing CLI capture is refused. An external bench timeout
of 150 seconds is appropriate for the 90-second example. Log wrapping can discard earlier data;
a completed file is not a completeness guarantee. No PID filter is used because it would hide
kernel messages.

This invokes `pnputil /restart-device`, not a guest reboot. Restart can fail for an adapter that
cannot be stopped; retain its exit/output and use boot capture below if necessary. An unchanged
Code 43 plus a successfully captured failure message is a useful result. An empty capture is
not proof that NVIDIA encountered no error, and a CLI-ready report does not establish that
NVIDIA emitted any messages.

## Earlier boot capture when restart is insufficient

```powershell
& .\debugview.ps1 -Action BootEnable
# Reboot this overlay using the bench harness, then collect:
& .\debugview.ps1 -Action Capture -Seconds 45
& .\debugview.ps1 -Action BootDisable
```

`BootEnable` saves only the previous DEFAULT and IHVVIDEO debug-filter registry values, enables
those masks, and invokes the signed tool's boot-start capture configuration. It **does not
reboot**. `BootDisable` removes boot capture and restores just those values; the snapshot remains
available if disable fails. Windows reads registry component masks at boot; the documented key
is `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Debug Print Filter`.
[Microsoft's filter description](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/reading-and-filtering-debugging-messages)
explains DEFAULT and IHVVIDEO. Query `-Action Status` to preserve boot/capture state. Whether
this driver's init failure produces an early message is still an experiment.

## Why this precedes a new observer callback

The pinned official NVIDIA 580.88 Windows driver at the controller has SHA256
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`. Its PE import table contains
`DbgPrintEx`, `vDbgPrintEx` and `KdDebuggerEnabled`, and strings contain NVRM/RPC errors. That
supports trying the system debug stream but does not prove a particular init error is printed.

OGKM 580.65.06 `src/nvidia/src/kernel/diagnostics/nvlog_printf.c:897` documents `RmMsg` rules,
including `:` for all prints. At `:1164`, `nvDbgInitRmMsg` reads `NV_REG_STR_RM_MSG`; the alternate
`NV_PRINTF_STRINGS_ALLOWED=0` build makes initialization empty and hides messages. The Windows
binary contains uppercase `RMMSG` but no exact `RmMsg` or `ResmanDebugLevel` string. Windows
registry names are case-insensitive; neither this string census nor the public Linux source
establishes which Windows registry path, if any, enables retail debug text. Windows-specific
registry routing is absent from this OGKM source. The wrapper therefore adds no speculative
NVIDIA logging keys; the signed capture tool is tested first.

Our pinned WDK's `wdm.h:15691` declares `PDEBUG_PRINT_CALLBACK` and `DbgSetDebugPrintCallback`.
However, the existing GSP observer's FAST_MUTEX-based FIFO is unsuitable for an arbitrary debug
callback: print calls can occur above APC_LEVEL, and callback recursion/unload synchronization
would need separate design. Microsoft documents [DbgPrint at IRQL up to DIRQL](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-dbgprint).
A new callback would require a preallocated, bounded, nonblocking buffer and explicit loss
accounting; it is deferred while a trusted signed tool already supplies capture.

## Evidence

`evidence/2026-10-05-prepare/` preserves the initial successful target preparation output.
Wrapper SHA256: `b7fab475aed5d4b5555095e5f33cbbaa0acfd8be746fe48c9759fce0cc307a12`.
The root bench recorded the command result in
`/data/kayfabe-runtime/windows-pool-20261005/probe-e-debugview-prepare.{stdout,stderr,json}`;
the target run is `C:\ProgramData\KayfabeDebugView\run-20261004T2352072534523Z`.
This proves preparation. [Runtime evidence](../../traces/windows_pool_20261005/probe-e-debugview/)
contains the subsequent successful
90-second capture, adapter restart and before/after device state from
`C:\ProgramData\KayfabeDebugView\run-20261004T2352435658653Z`. Windows E ran the integrated
Kayfabe revision `e71e4a8b` with pool and native timer experiments enabled. At 0.599 seconds,
WER records creation of `WATCHDOG-20261004-2352.dmp`. This establishes a diagnostic lead;
the debug text alone does not identify the driver failure's cause.

The corresponding private WER report identifies live-dump code `0x1b0`, first parameter `2`
and second parameter `0xc000009a`. Microsoft's
[0x1b0 reference](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x1b0--video-miniport-failed-livedump)
defines parameter 1 as the stage (`2`: StartDevice failure) and parameter 2 as the returned
NTSTATUS. `STATUS_INSUFFICIENT_RESOURCES` identifies that returned status; it does not establish
which resource or an actual system RAM shortage. This is a live diagnostic dump, not a crash.

## Bounded live-dump analysis

Use Microsoft's SDK KD for the kernel dump, following its documented
[kernel-dump workflow](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/analyzing-a-kernel-mode-dump-file-with-kd).
The controller prepares a small subset of official SDK 10.0.28000.2526 debugger files:

```sh
python3 tools/windows-debug-capture/fetch-debugger.py /tmp/kf-kd-bundle
```

This needs Python 3 and `7z`. It fetches five official CABs with SHA256 checks, extracts only
the 23 files in `debugger-manifest.json`, checks every extracted file and creates the exact
13 MiB pinned ZIP. No Windows executable runs on the controller. Transfer `kf-kd-bundle.zip`
and `analyze-dump.ps1` together to the Windows diagnostic overlay, parse the PowerShell script
before first use, then run:

```powershell
& .\analyze-dump.ps1 -DumpPath 'C:\Windows\LiveKernelReports\WATCHDOG\WATCHDOG-20261004-2348.dmp' -Seconds 240
```

The wrapper requires a canonical local WATCHDOG dump of at most 2 GiB; choose an existing
filename. WER may move a newer dump into its ReportQueue after capture. The wrapper creates
a fresh tool directory, verifies the bundle and each file, then requires valid Microsoft
Authenticode signatures for every executable/DLL before starting KD. An unused unsigned
downlevel compatibility shim is omitted; signature requirements were not relaxed. It analyzes
the dump as data with fixed command-file lines: `.symfix`, `.reload`, `.bugcheck`, `!ext.analyze -v`,
`lmvm nvlddmkm`, `kv`, `q`. It neither attaches to a running process nor changes the driver.
[`.symfix`](https://learn.microsoft.com/en-us/windows-hardware/drivers/debuggercmds/-symfix--set-symbol-store-path-)
selects Microsoft's symbol store. The analysis process is bounded by time and a polled
32 MiB output threshold; an outer bench timeout must also cover extraction/trust validation.
The script explicitly loads the bundled analysis extension and requires actual bugcheck
analysis/stack output and its completion marker. KD exit zero alone is insufficient: the
initial reduced package started but lacked `kext.dll`, so automatic analysis failed. The final
package includes that DLL and the analysis extension's dynamically loaded
`Microsoft.Diagnostics.Analysis.Utilities.dll` companion. Command-file lines avoid inline
semicolons being interpreted as path separators.

The raw dump remains private. Review debugger text before publishing; a dump can contain
unrelated memory. A timeout or missing private NVIDIA symbols limits the result and is not
proof of a root cause. The final bundle and wrapper passed on Windows E at
`C:\ProgramData\KayfabeKD\run-20261005T0011175941255Z`: all bundled PE signatures were valid,
KD exited zero, and the wrapper's semantic analysis check passed. Controller output is
preserved privately under `/data/kayfabe-runtime/windows-pool-20261005/kd-v4/` for review before
publication. A separate fresh controller fetch reproduced the pinned ZIP byte-for-byte.

| Artifact | SHA256 |
|---|---|
| [SDK bootstrap](https://download.microsoft.com/download/06fc99ac-527e-451e-a536-8866695a2e7e/KIT_BUNDLE_WINDOWSSDK_MEDIACREATION/winsdksetup.exe), inspected but not executed | `02988ea51eab2a2db53e19735e51c97a6d221ada74b9174fa0868870b9403ba0` |
| `SDK Debuggers-x86_en-us.msi`, inspected for x64 file/CAB mapping | `8f3467fe3982fef2f97b4c1d545b9eaab6f9d002b03fd2118bde4df615a94130` |
| Generated `kf-kd-bundle.zip` | `a3883456c4631bb46b9091a2d3197f17984df99ba89c36c2f8e54c976052d800` |
| x64 `kd.exe` | `30bb44fe1c3911bf5e43cf685d931097cbaa72bb2ff272eae9b0e016ee5c2ac9` |
| `analyze-dump.ps1` | `ada467b61279ee14ab3633a3bd7af7891de66dcfd600d432189cd18b02dc102e` |

The SDK version comes from [Microsoft's SDK downloads](https://learn.microsoft.com/en-us/windows/apps/windows-sdk/downloads).
Its MSI directory/component/file/media tables identify each member in the committed manifest;
bootstrap payload SHA1 checks were also matched during initial extraction, while the reusable
fetcher requires the stronger pinned SHA256 values. No SDK binaries are stored in this repo.
