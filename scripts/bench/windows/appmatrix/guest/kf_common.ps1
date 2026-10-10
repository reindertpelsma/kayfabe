# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_common.ps1 -- helpers shared by the app-matrix guest scripts (dot-sourced). Windows PowerShell 5.1
# compatible (the LTSC guest's inbox shell): no `??`, no ternary, no ConvertFrom-Json -AsHashtable.
$script:KF  = 'C:\kf'
$script:KFA = 'C:\kfapps'

function Initialize-KfDirs {
    foreach ($d in 'C:\kf', 'C:\kf\apps', 'C:\kf\results', 'C:\kf\logs', 'C:\kf\out', 'C:\kf\pycache', 'C:\kf\edge', 'C:\kf\py', 'C:\kf\media', 'C:\kfapps', 'C:\kfapps\.staged') {
        New-Item -ItemType Directory -Force -Path $d | Out-Null
    }
}

# write JSON atomically (a reader never sees half a file); UTF-8 (the host decodes utf-8-sig)
function Write-JsonAtomic {
    param($Object, [string]$Path, [int]$Depth = 8)
    $tmp = $Path + '.tmp'
    ($Object | ConvertTo-Json -Depth $Depth -Compress) | Set-Content -Path $tmp -Encoding UTF8
    Move-Item -Force -Path $tmp -Destination $Path
}

function Read-Json {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $null }
    return (Get-Content -Raw -Path $Path | ConvertFrom-Json)
}

# "00000000" -> "0", "0000d3a1" -> "d3a1"
function ConvertTo-LuidPart {
    param([string]$Hex)
    $t = $Hex.TrimStart('0').ToLower()
    if ($t.Length -eq 0) { return '0' }
    return $t
}

# the NVIDIA adapters' LUIDs ("high_low", normalised) written by kf_guest_setup.ps1 into C:\kf\adapters.json
function Get-NvidiaLuids {
    $a = Read-Json 'C:\kf\adapters.json'
    $r = @()
    if ($a -and $a.adapters) {
        foreach ($x in $a.adapters) {
            if ($x.vendor -eq 4318) { $r += ((ConvertTo-LuidPart $x.luid_high) + '_' + (ConvertTo-LuidPart $x.luid_low)) }
        }
    }
    return $r
}

# all descendants of a process (inclusive), from one CIM query
function Get-ProcessTreeIds {
    param([int]$RootPid)
    $all = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Select-Object ProcessId, ParentProcessId)
    $ids = New-Object System.Collections.Generic.List[int]
    $ids.Add($RootPid)
    $i = 0
    while ($i -lt $ids.Count) {
        $cur = $ids[$i]
        foreach ($p in $all) { if ($p.ParentProcessId -eq $cur -and -not $ids.Contains([int]$p.ProcessId)) { $ids.Add([int]$p.ProcessId) } }
        $i++
    }
    return ,$ids.ToArray()
}

# one sample of the per-process GPU Engine counters for the given pids: max utilisation per engine type,
# split into the NVIDIA adapter(s) and every other adapter (WARP / Basic Render shows up here)
function Get-GpuEngineSample {
    param([int[]]$Pids, [string[]]$NvLuids)
    $out = @{ nv = @{}; other = @{} }
    try { $cs = (Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples } catch { return $out }
    foreach ($c in $cs) {
        if ($c.InstanceName -match '^pid_(\d+)_luid_0x([0-9a-fA-F]+)_0x([0-9a-fA-F]+)_phys_(\d+)_eng_(\d+)_engtype_(.+)$') {
            $ppid = [int]$Matches[1]
            if ($Pids -notcontains $ppid) { continue }
            $luid = (ConvertTo-LuidPart $Matches[2]) + '_' + (ConvertTo-LuidPart $Matches[3])
            $et = $Matches[6]
            $v = [double]$c.CookedValue
            if ($NvLuids -contains $luid) { $tbl = $out.nv } else { $tbl = $out.other }
            if (-not $tbl.ContainsKey($et) -or $tbl[$et] -lt $v) { $tbl[$et] = $v }
        }
    }
    return $out
}

function Get-SmiPath {
    $p = Join-Path $env:SystemRoot 'System32\nvidia-smi.exe'
    if (Test-Path $p) { return $p }
    $c = Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue
    if ($c) { return $c.Source }
    return $null
}

# nvidia-smi utilisation / memory; $null when nvidia-smi is missing or fails
function Get-SmiSample {
    $smi = Get-SmiPath
    if (-not $smi) { return $null }
    try {
        $o = & $smi --query-gpu=utilization.gpu,memory.used,pstate --format=csv,noheader,nounits 2>&1 | Select-Object -First 1
        $f = ([string]$o).Split(',')
        if ($f.Count -ge 2) { return @{ util = [double]($f[0].Trim()); mem_mb = [double]($f[1].Trim()); pstate = $f[2].Trim() } }
    } catch { }
    return $null
}

# System-log events since $Since that matter for TDR / crash accounting (counts + the first lines)
function Get-GuestEventSummary {
    param([datetime]$Since)
    $r = @{ nvlddmkm_153 = 0; display_4101 = 0; wer_1001 = 0; kernel_power_41 = 0; unexpected_shutdown_6008 = 0; live_kernel = 0; bugcheck = 0; other_nvlddmkm = 0; lines = @() }
    try {
        $ev = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $Since } -ErrorAction SilentlyContinue)
    } catch { return $r }
    foreach ($e in $ev) {
        $pn = [string]$e.ProviderName
        $id = [int]$e.Id
        $msg = ([string]$e.Message -replace '\s+', ' ')
        $hit = $true
        if ($pn -match 'nvlddmkm') { if ($id -eq 153) { $r.nvlddmkm_153++ } else { $r.other_nvlddmkm++ } }
        elseif ($pn -eq 'Display' -and $id -eq 4101) { $r.display_4101++ }
        elseif ($pn -match 'Kernel-Power' -and $id -eq 41) { $r.kernel_power_41++ }
        elseif ($id -eq 6008) { $r.unexpected_shutdown_6008++ }
        elseif ($id -eq 1001 -and $msg -match 'LiveKernelEvent|WATCHDOG|0x00000116|0x00000117|0x00000141|VIDEO_TDR') { $r.wer_1001++; if ($msg -match 'LiveKernelEvent') { $r.live_kernel++ } else { $r.bugcheck++ } }
        elseif ($id -eq 1001 -and $msg -match 'bugcheck') { $r.wer_1001++; $r.bugcheck++ }
        else { $hit = $false }
        if ($hit -and $r.lines.Count -lt 16) {
            $m = $msg; if ($m.Length -gt 200) { $m = $m.Substring(0, 200) }
            $r.lines += ('{0} {1}/{2} {3}' -f $e.TimeCreated.ToUniversalTime().ToString('o'), $pn, $id, $m)
        }
    }
    return $r
}

function Get-BootTimeUtc {
    return (Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o')
}

# the volume of the app disk (label KFAPPS): its drive letter, or $null
function Get-AppDiskDrive {
    $v = Get-Volume -ErrorAction SilentlyContinue | Where-Object { $_.FileSystemLabel -eq 'KFAPPS' -and $_.DriveLetter } | Select-Object -First 1
    if ($v) { return ([string]$v.DriveLetter + ':') }
    return $null
}
