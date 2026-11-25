# Dump what LibreHardwareMonitor's library reads from the hardware as one JSON line (glue only; the judgement is hwmon_row.py).
#   lhm_dump.ps1 -LibDir <the unpacked release zip> [-GpuEnabled 1|0]
# -GpuEnabled 0 is the negative control of the GPU-use assertion: the same reader with the GPU group switched off must see no NVIDIA hardware.
param([Parameter(Mandatory = $true)][string]$LibDir, [int]$GpuEnabled = 1)
$ErrorActionPreference = 'Stop'
# the zip's LibreHardwareMonitor.exe.config redirects assembly versions for the exe; this process is powershell.exe, so every dependency is taken from the zip's directory
[AppDomain]::CurrentDomain.add_AssemblyResolve({
    param($sender, $e)
    $p = Join-Path $LibDir ((New-Object System.Reflection.AssemblyName($e.Name)).Name + '.dll')
    if (Test-Path -LiteralPath $p) { return [System.Reflection.Assembly]::LoadFrom($p) }
    return $null
})
$asm = [System.Reflection.Assembly]::LoadFrom((Join-Path $LibDir 'LibreHardwareMonitorLib.dll'))
$out = [ordered]@{ lib_version = $asm.GetName().Version.ToString(); gpu_enabled = [bool]$GpuEnabled; hardware = @(); errors = @() }
function Read-Hardware($h) {
    $h.Update()
    $sensors = @()
    foreach ($s in $h.Sensors) { $sensors += [ordered]@{ type = $s.SensorType.ToString(); name = $s.Name; value = $s.Value } }
    $sub = @()
    foreach ($x in $h.SubHardware) { $sub += Read-Hardware $x }
    [ordered]@{ type = $h.HardwareType.ToString(); name = $h.Name; sensors = $sensors; sub = $sub }
}
try {
    $c = New-Object LibreHardwareMonitor.Hardware.Computer
    $c.IsGpuEnabled = [bool]$GpuEnabled
    $c.Open()
    foreach ($h in $c.Hardware) { $out.hardware += Read-Hardware $h }
    $c.Close()
} catch { $out.errors += $_.Exception.GetType().FullName + ': ' + $_.Exception.Message }
$out | ConvertTo-Json -Depth 8 -Compress
