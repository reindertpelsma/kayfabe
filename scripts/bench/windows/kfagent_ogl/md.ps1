param([int]$ProcId, [string]$Out, [int]$Type = 0x1 + 0x4 + 0x20 + 0x800)
# MiniDumpWithDataSegs|HandleData|UnloadedModules|ThreadInfo ; no full memory
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class KfMd { [DllImport("dbghelp.dll", SetLastError=true)] public static extern bool MiniDumpWriteDump(IntPtr h, uint pid, IntPtr f, int t, IntPtr e, IntPtr u, IntPtr c); }
"@
$p = [Diagnostics.Process]::GetProcessById($ProcId)
$fs = [IO.File]::Create($Out)
$ok = [KfMd]::MiniDumpWriteDump($p.Handle, [uint32]$ProcId, $fs.SafeFileHandle.DangerousGetHandle(), $Type, [IntPtr]::Zero, [IntPtr]::Zero, [IntPtr]::Zero)
$fs.Close(); "MINIDUMP ok=$ok size=$((Get-Item $Out).Length) at $((Get-Date).ToString('HH:mm:ss.fff'))"
