# Keep only model/driver data. No unfiltered diagnostic XML is written.
param([Parameter(Mandatory=$true)][string]$Out)
$ErrorActionPreference = 'Stop'
$controllers = @(Get-CimInstance Win32_VideoController |
    Select-Object Name, DriverVersion, VideoProcessor)
$nvidia = @()
if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
    $xml = & nvidia-smi -q -x 2>$null
    if ($LASTEXITCODE -eq 0) {
        # Drop the external DOCTYPE before loading the in-memory document.
        [xml]$info = ($xml | Where-Object { $_ -notmatch '^\s*<!DOCTYPE' }) -join "`n"
        $nvidia = @($info.nvidia_smi_log.gpu | Select-Object product_name, product_architecture)
    }
}
@{windows = $controllers; nvidia = $nvidia} | ConvertTo-Json -Depth 4 |
    Set-Content -Encoding utf8 -LiteralPath $Out
