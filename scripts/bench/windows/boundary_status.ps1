# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$os = Get-CimInstance Win32_OperatingSystem
$service = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
$driver = ([string]$service.ImagePath).Trim('"') -replace '^\\SystemRoot\\',"$env:windir\"
$driverHash = (Get-FileHash $driver -Algorithm SHA256).Hash.ToLowerInvariant()
$smi = "$env:windir\System32\nvidia-smi.exe"
if (!(Test-Path $smi)) { $smi = "$env:ProgramFiles\NVIDIA Corporation\NVSMI\nvidia-smi.exe" }
$result = [ordered]@{available=(Test-Path $smi); exit_code=$null; stdout=''; stderr=''; timed_out=$false}
if ($result.available) {
    $p = [Diagnostics.Process]::new()
    $p.StartInfo.FileName = $smi
    $p.StartInfo.Arguments = '--query-gpu=name,driver_version --format=csv,noheader'
    $p.StartInfo.UseShellExecute = $false
    $p.StartInfo.RedirectStandardOutput = $true
    $p.StartInfo.RedirectStandardError = $true
    if (!$p.Start()) { throw 'Could not launch nvidia-smi' }
    $out = $p.StandardOutput.ReadToEndAsync()
    $err = $p.StandardError.ReadToEndAsync()
    if (!$p.WaitForExit(30000)) { $result.timed_out=$true; $p.Kill(); $p.WaitForExit() }
    $result.exit_code=$p.ExitCode
    $result.stdout=$out.GetAwaiter().GetResult()
    $result.stderr=$err.GetAwaiter().GetResult()
    $p.Dispose()
}
[ordered]@{
    time_utc=[DateTime]::UtcNow.ToString('o')
    uptime_seconds=([DateTime]::Now-$os.LastBootUpTime).TotalSeconds
    os_version=$os.Version
    driver_sha256=$driverHash
    driver_file_version=(Get-Item $driver).VersionInfo.FileVersion
    display=@(Get-CimInstance Win32_PnPEntity | Where-Object PNPClass -eq 'Display' |
        Select-Object Name,PNPDeviceID,ConfigManagerErrorCode,Status)
    nvidia_smi=$result
} | ConvertTo-Json -Depth 6
