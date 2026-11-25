# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
<#
.SYNOPSIS
    The few guest settings a Windows guest needs to run NVIDIA's stock driver on kayfabe.

.DESCRIPTION
    Run in the guest from an elevated PowerShell (or through kayfabe-windows-setup.bat), ideally BEFORE the NVIDIA
    driver installer, so that the display is not lost after the install. The list of what it sets is in
    kayfabe-windows-setup.json (the $KfSettings table below is the same list; a test keeps them equal).

    Two phases, because the display adapter's class key (HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-...}\<index>)
    only exists once a driver is bound to the NVIDIA device:
      pre   sets what can be set before the install: the nvlddmkm service key (created if missing; a key that holds
            only EnableGpuFirmware has no ImagePath or Type, so it is not a service, and the installer's AddService
            fills it in) and the Dwm value.
      post  after the NVIDIA driver is installed: reads the adapter's driver key (DEVPKEY_Device_Driver of the PCI
            vendor 10DE device), sets the class-key value (and re-applies the pre values), then asks for a restart.
      auto  (default) post when an NVIDIA device is bound to nvlddmkm, otherwise pre. A device on Microsoft's Basic
            Display driver is not bound to NVIDIA's driver.

    -InstallDriver <dir> does it in one step (the order scripts/bench/windows/gsp_on.ps1 uses): pre values, disable the
    device, add and install the INF, write the class-key value, read back, re-enable the device.

    Every value is read back after it is written; a value that does not read back fails the run. Running it again
    changes nothing that is already right. Nothing is printed except the keys, the values and device ids of the
    form VEN_10DE&DEV_xxxx. It never restarts the machine.

    Exit codes: 0 done, 1 a value did not read back or could not be written, 2 refused (not elevated, bad
    arguments), 3 no NVIDIA device, or no NVIDIA driver bound to it where one is needed.

.PARAMETER Phase
    auto (default), pre or post.

.PARAMETER InstallDriver
    The directory the NVIDIA driver package was extracted to (it holds Display.Driver\nv_dispi.inf).

.PARAMETER DriverKey
    The adapter's driver key as Windows reports it, for example {4d36e968-e325-11ce-bfc1-08002be10318}\0001, when
    discovery is not wanted. Only the display class and a four-digit index are accepted.
#>
[CmdletBinding()]
param(
    [ValidateSet('auto', 'pre', 'post')][string]$Phase = 'auto',
    [string]$InstallDriver = '',
    [string]$DriverKey = ''
)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'

# The single list of what is set. Keep equal to kayfabe-windows-setup.json (scripts/ci/test_windows_guest_setup.py).
$KfSettings = @(
    @{ Id = 1; Phase = 'post'; Path = 'SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>'; Name = 'EnableGpuFirmware'; Type = 'DWord'; Value = 1 },
    @{ Id = 2; Phase = 'pre'; Path = 'SYSTEM\CurrentControlSet\Services\nvlddmkm'; Name = 'EnableGpuFirmware'; Type = 'DWord'; Value = 1 },
    @{ Id = 3; Phase = 'pre'; Path = 'SOFTWARE\Microsoft\Windows\Dwm'; Name = 'OverlayTestMode'; Type = 'DWord'; Value = 5 }
)

$SayLog = New-Object System.Collections.Generic.List[string]

function Say([string]$Text) {
    $SayLog.Add($Text)
    Write-Host "KAYFABE-SETUP $Text"
}

# ---- primitives (a test replaces these with an in-memory registry and a fake device list) ----

function Test-Elevated {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    return ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Test-RegKey([string]$Path) {
    return (Test-Path -LiteralPath "HKLM:\$Path")
}

function Get-RegDword([string]$Path, [string]$Name) {
    $p = Get-ItemProperty -LiteralPath "HKLM:\$Path" -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $p) { return $null }
    return $p.$Name
}

function Set-RegDword([string]$Path, [string]$Name, [int]$Value) {
    if (-not (Test-RegKey $Path)) { New-Item -Path "HKLM:\$Path" -Force | Out-Null }
    New-ItemProperty -LiteralPath "HKLM:\$Path" -Name $Name -PropertyType DWord -Value $Value -Force | Out-Null
}

function Get-NvidiaDevice {
    $all = @(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&*' })
    $display = @($all | Where-Object { $_.Class -eq 'Display' })
    if ($display.Count -gt 0) { return $display[0] }
    $other = @($all | Where-Object { $_.Class -ne 'MEDIA' })
    if ($other.Count -gt 0) { return $other[0] }
    return $null
}

function Get-DeviceProp([string]$InstanceId, [string]$KeyName) {
    $p = Get-PnpDeviceProperty -InstanceId $InstanceId -KeyName $KeyName -ErrorAction SilentlyContinue
    if ($null -eq $p) { return '' }
    if ($null -eq $p.Data) { return '' }
    return [string]$p.Data
}

function Invoke-Pnputil([string[]]$Arguments) {
    $out = & "$env:SystemRoot\System32\pnputil.exe" @Arguments 2>&1
    foreach ($line in $out) { Say "pnputil: $line" }
    return $LASTEXITCODE
}

# ---- logic ----

function Get-NvidiaState {
    $state = @{ Present = $false; InstanceId = ''; Short = ''; Service = ''; DriverKey = ''; Bound = $false }
    $dev = Get-NvidiaDevice
    if ($null -eq $dev) { return $state }
    $state.Present = $true
    $state.InstanceId = $dev.InstanceId
    if ($dev.InstanceId -match 'VEN_10DE&DEV_[0-9A-Fa-f]{4}') { $state.Short = $Matches[0] }
    $state.Service = Get-DeviceProp $dev.InstanceId 'DEVPKEY_Device_Service'
    $state.DriverKey = Get-DeviceProp $dev.InstanceId 'DEVPKEY_Device_Driver'
    $state.Bound = ($state.Service -eq 'nvlddmkm') -and ($state.DriverKey -ne '')
    return $state
}

function Test-DriverKeyShape([string]$Key) {
    return ($Key -match '^\{4d36e968-e325-11ce-bfc1-08002be10318\}\\[0-9]{4}$')
}

function Resolve-SettingPath($Setting, [string]$Key) {
    if ($Setting.Path.EndsWith('<index>')) {
        return $Setting.Path.Replace('<index>', $Key.Substring($Key.LastIndexOf('\') + 1))
    }
    return $Setting.Path
}

# Writes one value (only when it differs), then reads it back. Returns $true only if the read-back equals the value.
function Set-VerifiedDword([string]$Path, [string]$Name, [int]$Value) {
    $was = Get-RegDword $Path $Name
    if ($null -ne $was -and $was -eq $Value) {
        Say "already  HKLM\$Path $Name=$Value"
    } else {
        $wasText = '<absent>'
        if ($null -ne $was) { $wasText = [string]$was }
        try {
            Set-RegDword $Path $Name $Value
            Say "set      HKLM\$Path $Name=$Value (was $wasText)"
        } catch {
            Say "FAILED   HKLM\$Path $Name could not be written: $($_.Exception.Message)"
            return $false
        }
    }
    $now = Get-RegDword $Path $Name
    if ($null -ne $now -and $now -eq $Value) {
        Say "readback HKLM\$Path $Name=$now OK"
        return $true
    }
    $nowText = '<absent>'
    if ($null -ne $now) { $nowText = [string]$now }
    Say "FAILED   HKLM\$Path $Name reads back $nowText, expected $Value"
    return $false
}

function Set-PhaseSettings([string[]]$Phases, [string]$Key) {
    $ok = $true
    foreach ($s in $KfSettings) {
        if ($Phases -notcontains $s.Phase) { continue }
        if ($s.Path.EndsWith('<index>') -and -not $Key) { continue }
        $path = Resolve-SettingPath $s $Key
        if (-not (Set-VerifiedDword $path $s.Name $s.Value)) { $ok = $false }
    }
    return $ok
}

function Show-Refusal([string]$Text) {
    Say "REFUSED $Text"
}

function Find-DriverInf([string]$Dir) {
    $direct = Join-Path $Dir 'Display.Driver\nv_dispi.inf'
    if (Test-Path -LiteralPath $direct) { return $direct }
    $flat = Join-Path $Dir 'nv_dispi.inf'
    if (Test-Path -LiteralPath $flat) { return $flat }
    $found = @(Get-ChildItem -LiteralPath $Dir -Filter 'nv_dispi.inf' -Recurse -ErrorAction SilentlyContinue)
    if ($found.Count -gt 0) { return $found[0].FullName }
    return ''
}

function Invoke-Pre {
    Say 'phase pre: settings that can be set before the NVIDIA driver is installed'
    $ok = Set-PhaseSettings @('pre') ''
    if (-not $ok) { Say 'FAILED   pre phase: at least one value did not read back'; return 1 }
    Say 'done     pre phase. Next: install the NVIDIA driver, then run this script again (phase post), then restart Windows.'
    Say 'note     the display adapter class key is created by the driver install; the post phase sets it.'
    return 0
}

function Invoke-Post([string]$Key) {
    Say 'phase post: the NVIDIA driver is installed'
    if (-not $Key) {
        $state = Get-NvidiaState
        if (-not $state.Present) {
            Show-Refusal 'no NVIDIA PCI device (vendor 10DE) is present. Nothing was changed.'
            return 3
        }
        if (-not $state.Bound) {
            Show-Refusal "$($state.Short) is not bound to NVIDIA's driver (service '$($state.Service)'). Install the NVIDIA driver first, then run this again. Nothing was changed."
            return 3
        }
        $Key = $state.DriverKey
        Say "device   $($state.Short) bound to nvlddmkm, driver key $Key"
    } else {
        Say "device   driver key given on the command line: $Key"
    }
    if (-not (Test-DriverKeyShape $Key)) {
        Show-Refusal "the driver key '$Key' is not a display-class key of the form {4d36e968-e325-11ce-bfc1-08002be10318}\0000. Nothing was changed."
        return 2
    }
    $ok = Set-PhaseSettings @('pre', 'post') $Key
    if (-not $ok) { Say 'FAILED   post phase: at least one value did not read back'; return 1 }
    Say 'done     post phase. RESTART REQUIRED: restart Windows now (shutdown /r /t 0) so the driver starts with the settings.'
    return 0
}

function Invoke-Install([string]$Dir) {
    if (-not (Test-Path -LiteralPath $Dir -PathType Container)) {
        Show-Refusal "-InstallDriver: '$Dir' is not a directory. Nothing was changed."
        return 2
    }
    $inf = Find-DriverInf $Dir
    if (-not $inf) {
        Show-Refusal "-InstallDriver: no nv_dispi.inf under '$Dir'. Extract NVIDIA's package first (7-Zip opens the installer). Nothing was changed."
        return 2
    }
    $state = Get-NvidiaState
    if (-not $state.Present) {
        Show-Refusal 'no NVIDIA PCI device (vendor 10DE) is present. Nothing was changed.'
        return 3
    }
    Say "install  device $($state.Short), inf $inf"
    Say 'step 1   settings that can be set before the install'
    if (-not (Set-PhaseSettings @('pre') '')) {
        Say 'FAILED   a pre value did not read back; the device was not touched.'
        return 1
    }
    Say 'step 2   disable the device'
    $rc = Invoke-Pnputil @('/disable-device', $state.InstanceId)
    Say "rc       disable-device $rc"
    Say 'step 3   add and install the INF'
    $rc = Invoke-Pnputil @('/add-driver', $inf, '/install')
    Say "rc       add-driver $rc"
    $state = Get-NvidiaState
    if ($state.Bound) {
        Say "step 4   driver key $($state.DriverKey), writing the class-key value before the first start"
        if (-not (Set-PhaseSettings @('pre', 'post') $state.DriverKey)) {
            Say 'FAILED   a value did not read back. The device is LEFT DISABLED so the driver does not start without it.'
            Say "recover  fix the value (see above), then: pnputil /enable-device `"$($state.InstanceId)`""
            return 1
        }
        Say 'step 5   enable the device'
        $rc = Invoke-Pnputil @('/enable-device', $state.InstanceId)
        Say "rc       enable-device $rc"
        Say 'done     driver installed with the settings in place. Restart Windows to be safe (shutdown /r /t 0).'
        return 0
    }
    Say "WARN     the device is still on '$($state.Service)': pnputil skipped the disabled device. Enabling it; if Windows picks the NVIDIA driver now, run this script again (phase post) and restart."
    $rc = Invoke-Pnputil @('/enable-device', $state.InstanceId)
    Say "rc       enable-device $rc"
    $state = Get-NvidiaState
    if ($state.Bound) {
        if (-not (Set-PhaseSettings @('pre', 'post') $state.DriverKey)) { return 1 }
        Say 'done     the driver bound after the enable and the class-key value is now set. RESTART REQUIRED.'
        return 0
    }
    return 3
}

function Invoke-Setup([string]$Phase, [string]$InstallDriver, [string]$DriverKey) {
    if (-not (Test-Elevated)) {
        Show-Refusal 'this needs an elevated PowerShell (Run as administrator). Nothing was changed.'
        return 2
    }
    if ($InstallDriver -and $Phase -ne 'auto') {
        Show-Refusal '-InstallDriver does its own phases; do not combine it with -Phase. Nothing was changed.'
        return 2
    }
    if ($InstallDriver) { return (Invoke-Install $InstallDriver) }
    if ($Phase -eq 'auto') {
        $state = Get-NvidiaState
        if ($state.Bound -or $DriverKey) {
            $Phase = 'post'
            Say 'phase    auto -> post (an NVIDIA device is bound to nvlddmkm)'
        } else {
            $Phase = 'pre'
            Say 'phase    auto -> pre (no NVIDIA device is bound to the NVIDIA driver yet)'
        }
    }
    if ($Phase -eq 'pre') { return (Invoke-Pre) }
    return (Invoke-Post $DriverKey)
}

# Dot-sourcing (a test) only defines the functions above.
if ($MyInvocation.InvocationName -eq '.') { return }

exit (Invoke-Setup $Phase $InstallDriver $DriverKey)
