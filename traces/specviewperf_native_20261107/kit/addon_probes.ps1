# addon_probes.ps1 -- native reference for two winapps probes (interactive session): nbody_demo_gl (25 s alive + GPU engine counters) and
# kf_overlayprobe (steady NV12 overlay scenario). Prints ADDON lines; GPU counters are the PDH "GPU Engine" instances (what the lane's
# `pdh:Cuda|Compute|3D|Copy` proof reads). Run after C:\kf\addon has demo.zip, cudart.zip and kf_overlayprobe.exe.
$ErrorActionPreference = 'Continue'
$a = 'C:\kf\addon'
Expand-Archive "$a\demo.zip" "$a\demo" -Force
Expand-Archive "$a\cudart.zip" "$a\cudart" -Force
$d = Get-ChildItem "$a\demo" -Directory | Select -First 1
$demo = "$($d.FullName)\demo_suite"
Copy-Item (Get-ChildItem "$a\cudart" -Recurse -Filter cudart64_12.dll | Select -First 1).FullName $demo -Force
Add-Type -AssemblyName System.Windows.Forms
$scr = ([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { $_.DeviceName + ' ' + $_.Bounds }) -join '; '
"ADDON screens: $scr"
# --- nbody
$p = Start-Process -FilePath "$demo\nbody.exe" -ArgumentList '-numbodies=16384' -WorkingDirectory $demo -PassThru
$null = $p.Handle
$max = @{}
for ($i = 0; $i -lt 25; $i++) {
    Start-Sleep -Seconds 1
    if ($p.HasExited) { break }
    if ($i % 3 -eq 2) {
        try {
            (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples | Where-Object { $_.CookedValue -gt 0 } | ForEach-Object {
                $t = if ($_.InstanceName -match 'engtype_(\w+)') { $Matches[1] } else { 'other' }
                if (-not $max.ContainsKey($t) -or $max[$t] -lt $_.CookedValue) { $max[$t] = [math]::Round($_.CookedValue, 1) }
            }
        } catch { "ADDON counter_error $_" }
    }
}
if ($p.HasExited) { "ADDON nbody EXITED_EARLY rc=$($p.ExitCode) after=$i s" } else { "ADDON nbody ALIVE_AT 25 s"; & taskkill.exe /PID $p.Id /T /F 2>&1 | Out-Null }
"ADDON nbody gpu_engine_max_util_pct " + (($max.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ' ')
& "$env:windir\System32\nvidia-smi.exe" --query-gpu=utilization.gpu,memory.used --format=csv,noheader | ForEach-Object { "ADDON nbody smi_after $_" }
# --- overlay probe
$out = "$a\overlay.json"
Remove-Item $out -ErrorAction SilentlyContinue
$o = & "$a\kf_overlayprobe.exe" --scenario steady --duration 30 --out $out 2>&1
$o | ForEach-Object { [string]$_ } | Where-Object { $_ -notmatch '^KFOVL JSON ' } | Select -First 60 | ForEach-Object { "ADDON overlay $_" }
"ADDON overlay rc=$LASTEXITCODE"
if (Test-Path $out) { "ADDON overlay json_bytes=" + (Get-Item $out).Length }
