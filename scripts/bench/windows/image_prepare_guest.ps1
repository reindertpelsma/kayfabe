# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Run elevated over win_vm.sh's existing guest channel before a full shutdown.
# No downloads. See docs/design/V3_WINDOWS_IMAGE.md for the bounded zero-fill fallback.
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^image_[0-9TZ]+$')][string]$SessionId,
    [string]$ConfigPath = 'C:\kf\image-config.json',
    [string]$OutputPath = 'C:\kf\image-inventory.json',
    [string]$ToolsDir = 'C:\kf\tools'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
try {
    # An interrupted or failed preparation must not leave a reusable success marker.
    if (Test-Path -LiteralPath $OutputPath) { Remove-Item -LiteralPath $OutputPath -Force }
    $config = Get-Content -LiteralPath $ConfigPath -Raw | ConvertFrom-Json
    if ($null -eq $config.app_roots -or $null -eq $config.exclude) { throw 'Missing config arrays' }
    foreach ($exclusion in $config.exclude) {
        if (-not $exclusion.pattern -or -not $exclusion.reason) { throw 'Exclusion needs pattern and reason' }
    }

    powercfg.exe /h off | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'powercfg failed' }
    $power = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power'
    Set-ItemProperty -LiteralPath $power -Name HiberbootEnabled -Type DWord -Value 0
    if ((Get-ItemProperty -LiteralPath $power -Name HiberbootEnabled).HiberbootEnabled -ne 0) {
        throw 'Fast startup still enabled'
    }
    if (Test-Path -LiteralPath "$env:SystemDrive\hiberfil.sys") { throw 'Hibernation file still exists' }
    Stop-Service -Name W32Time -Force
    Set-Service -Name W32Time -StartupType Disabled
    if ((Get-Service -Name W32Time).Status -ne 'Stopped' -or
        (Get-CimInstance Win32_Service -Filter "Name='W32Time'").StartMode -ne 'Disabled') {
        throw 'Windows time service still active'
    }
    $encryption = Get-BitLockerVolume -MountPoint $env:SystemDrive
    if ([string]$encryption.VolumeStatus -ne 'FullyDecrypted' -or [string]$encryption.ProtectionStatus -ne 'Off') {
        throw 'Image volume must be fully decrypted with BitLocker off'
    }

    $registry = @(
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
    )
    $uninstall = @(
        foreach ($key in $registry) {
            if (Test-Path -LiteralPath $key) {
                foreach ($entry in Get-ChildItem -LiteralPath $key) {
                    $props = Get-ItemProperty -LiteralPath $entry.PSPath
                    # Select-Object tolerates optional values under StrictMode.
                    $row = $props | Select-Object DisplayName, DisplayVersion, Publisher, InstallLocation
                    if ($row.DisplayName) {
                        [ordered]@{ registry = $key; name = $row.DisplayName; version = $row.DisplayVersion
                                    publisher = $row.Publisher; install_location = $row.InstallLocation }
                    }
                }
            }
        }
    )
    $packages = @(Get-Package | Select-Object Name, Version, ProviderName, Source | Sort-Object Name, Version)
    $drivers = @(Get-CimInstance Win32_PnPSignedDriver |
        Where-Object { $_.DriverProviderName -match 'NVIDIA' } |
        Select-Object DeviceName, DriverVersion, DriverDate, InfName | Sort-Object DeviceName)
    $version = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $windows = [ordered]@{ product = $version.ProductName; build = $version.CurrentBuild
                           ubr = $version.UBR; edition = $version.EditionID }
    $files = New-Object 'System.Collections.Generic.List[object]'
    $excluded = New-Object 'System.Collections.Generic.List[object]'
    $roots = New-Object 'System.Collections.Generic.List[string]'
    foreach ($chosen in $config.app_roots) {
        $root = Get-Item -LiteralPath $chosen
        if (-not $root.PSIsContainer -or ($root.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'App root must be a real directory'
        }
        $roots.Add($root.FullName)
        foreach ($file in Get-ChildItem -LiteralPath $root.FullName -Recurse -Force | Sort-Object FullName) {
            if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Reparse point inside app root' }
            if ($file.PSIsContainer) { continue }
            $reason = $null
            foreach ($rule in $config.exclude) {
                if ($file.FullName -like $rule.pattern) { $reason = $rule.reason; break }
            }
            if ($reason) {
                $excluded.Add([ordered]@{ path = $file.FullName; reason = $reason })
            } else {
                $hash = Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256
                $files.Add([ordered]@{ path = $file.FullName; bytes = $file.Length; sha256 = $hash.Hash.ToLowerInvariant() })
            }
        }
    }

    # ZERO_FILL_START
    # Prefer an owner-staged Sysinternals binary; never fetch or silently accept its EULA.
    $sdelete = @('sdelete64.exe', 'sdelete.exe') | ForEach-Object { Join-Path $ToolsDir $_ } |
        Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if ($sdelete) {
        & $sdelete '-z' $env:SystemDrive | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'sdelete failed; owner must accept the official EULA interactively first' }
        $zero = [ordered]@{ method = 'sdelete'; completed = $true; reserve_bytes = 0 }
    } else {
        # Write actual zero bytes, never SetLength (which could create sparse extents).
        # Leave 64 MiB so Windows/SSH can finish; that tail is explicitly not claimed zeroed.
        $zeroPath = Join-Path ($env:SystemDrive + '\') 'kf-image-zero.tmp'
        $stream = $null
        $ownedZero = $false
        $written = [long]0
        $reserve = [long](64MB)
        try {
            $stream = [IO.File]::Open($zeroPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            $ownedZero = $true
            $buffer = New-Object byte[] (1MB)
            $drive = New-Object IO.DriveInfo ($env:SystemDrive + '\')
            while ($drive.AvailableFreeSpace -gt $reserve) {
                $count = [int][Math]::Min($buffer.Length, $drive.AvailableFreeSpace - $reserve)
                if ($count -le 0) { break }
                $stream.Write($buffer, 0, $count)
                $written += $count
            }
            $stream.Flush($true)
        } finally {
            if ($null -ne $stream) { $stream.Dispose() }
            if ($ownedZero) { Remove-Item -LiteralPath $zeroPath -Force }
        }
        $zero = [ordered]@{ method = 'zero-stream-with-reserve'; completed = $true
                            reserve_bytes = $reserve; bytes_written = $written }
    }

    # INVENTORY_COMMIT: successful JSON is produced only after every preparation step.
    $inventory = [ordered]@{
        schema = 'kayfabe-windows-inventory/1'; session_id = $SessionId
        prepared = $true; hibernation_disabled = $true; fast_startup_disabled = $true
        time_service_disabled = $true; bitlocker_decrypted = $true; zero_fill = $zero
        windows = $windows; drivers = $drivers; packages = $packages; uninstall_registry = $uninstall
        app_roots = @($roots.ToArray()); files = @($files.ToArray()); exclusions = @($excluded.ToArray())
        exclusion_rules = @($config.exclude); collected_at_guest_clock = (Get-Date).ToUniversalTime().ToString('o')
    }
    $inventory | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath ($OutputPath + '.tmp') -Encoding UTF8
    Move-Item -LiteralPath ($OutputPath + '.tmp') -Destination $OutputPath -Force
    Write-Output 'IMAGE_PREPARE_DONE'
    exit 0
} catch {
    # Do not echo arbitrary registry/package/file contents into the host's logs.
    Write-Error 'Image preparation failed; no completed inventory was published' -ErrorAction Continue
    exit 1
}
