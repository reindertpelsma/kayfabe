# Windows kernel diagnostics for the Kayfabe boot failure

**STATUS: RESEARCH, 2026-10-05.** The pinned tool and wrapper passed Windows preparation on the
Kayfabe Windows E overlay: PowerShell parser clean, executable hash and Microsoft Authenticode
valid, actual CLI help/version and required options checked. Kernel capture is still in progress. This supplements GSP traffic with whatever diagnostics the
Windows driver actually emits. It does not change Kayfabe policy or patch `nvlddmkm.sys`.

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
CSV plus stdout/stderr, tool identity, command exit, adapter state and the restart result.
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

## Preparation evidence

`evidence/2026-10-05-prepare/` preserves the initial successful target preparation output.
Wrapper SHA256: `b7fab475aed5d4b5555095e5f33cbbaa0acfd8be746fe48c9759fce0cc307a12`.
The root bench recorded the command result in
`/data/kayfabe-runtime/windows-pool-20261005/probe-e-debugview-prepare.{stdout,stderr,json}`;
the target run is `C:\ProgramData\KayfabeDebugView\run-20261004T2352072534523Z`.
This proves preparation, not that kernel records or the NVIDIA failure were captured.
