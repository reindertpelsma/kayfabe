# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Shared guest policy and provider. Windows APIs are UNTESTED; fixtures never call them.
Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'
$script:KfFixture = $null
$script:KfActions = New-Object 'System.Collections.Generic.List[object]'
$script:KfProtected = @{}
$script:KfServiceFloor = @('NVDisplay.Container*','NvContainerLocalSystem','NvContainerNetworkService',
    'sshd','QEMU-GA','qemu-ga','msiserver','TBS','CryptSvc','KeyIso','VaultSvc','NgcSvc','NgcCtnrSvc',
    'RpcSs','RpcEptMapper','DcomLaunch','PlugPlay','Power','EventLog','Schedule','Winmgmt','LSM','SamSs',
    'ProfSvc','UserManager','BrokerInfrastructure','CoreMessagingRegistrar','SystemEventsBroker',
    'StateRepository','Dhcp','Dnscache','NlaSvc','nsi','Netman','NetSetupSvc','BFE','MpsSvc',
    'WinDefend','WdNisSvc','SecurityHealthService','wscsvc','sppsvc','LicenseManager','ClipSVC','AppXSvc',
    'Audiosrv','AudioEndpointBuilder','Themes','DispBrokerDesktopSvc','DisplayEnhancementService')
$script:KfAlwaysOff = @('wuauserv','UsoSvc','WaaSMedicSvc','BITS','DoSvc','DiagTrack','dmwappushservice','WSearch','W32Time')

function Assert-KfWindows {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'windows-provider-unavailable' }
}
function Get-KfFailureReason($ErrorRecord) {
    $message=[string]$ErrorRecord.Exception.Message
    # Only our bounded policy codes may reach logs; never raw registry/task data.
    if ($message -cmatch '^[a-z][a-z0-9-]{1,79}$') { return $message }
    return 'provider-or-input-failure'
}
function Get-KfProp($Object, [string]$Name, $Default = $null) {
    $p = $Object.PSObject.Properties[$Name]
    if ($null -eq $p) { return $Default }
    return $p.Value
}
function Assert-KfFields($Object, [string[]]$Required, [string[]]$Optional = @()) {
    if ($null -eq $Object -or $Object -is [string] -or $Object -is [Array]) { throw 'object-required' }
    $names = @($Object.PSObject.Properties.Name)
    foreach ($n in $Required) { if ($names -notcontains $n) { throw 'missing-field' } }
    foreach ($n in $names) { if (@($Required + $Optional) -notcontains $n) { throw 'unknown-field' } }
}
function Assert-KfVersion($Object, [string]$Kind) {
    if (($Object.schema_version -isnot [int] -and $Object.schema_version -isnot [long]) -or
        $Object.schema_version -ne 1 -or $Object.kind -cne $Kind) { throw 'unsupported-schema' }
}
function Assert-KfId([string]$Id) {
    if ($Id -cnotmatch '^[a-z][a-z0-9_]{0,63}$') { throw 'invalid-app-id' }
}
function Read-KfJson([string]$Path) {
    $f = Get-Item -LiteralPath $Path
    if ($f.Length -gt 32MB -or ($f.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'invalid-json-file' }
    return (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json)
}
function Write-KfJson($Object, [string]$Path) {
    $parent = Split-Path -Parent $Path
    if ($parent -and -not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Path $parent | Out-Null }
    $temp = $Path + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    try {
        $Object | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $temp -Encoding UTF8
        Move-Item -LiteralPath $temp -Destination $Path -Force
    } finally { if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp } }
}
function Get-KfHash([string]$Path) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Initialize-KfBackend([string]$FixturePath) {
    if ($FixturePath) {
        $script:KfFixture = Read-KfJson $FixturePath
        Assert-KfVersion $script:KfFixture 'fixture'
    } else { Assert-KfWindows }
}
function Save-KfFixture([string]$Path) {
    if ($Path -and $null -ne $script:KfFixture) {
        $script:KfFixture.actions = @($script:KfActions.ToArray())
        Write-KfJson $script:KfFixture $Path
    }
}
function Add-KfAction([string]$Op, [string]$Id) {
    if ($null -ne $script:KfFixture -and @($script:KfFixture.fail_on) -contains ($Op + ':' + $Id)) { throw 'fixture-operation-failed' }
    $script:KfActions.Add([ordered]@{ op = $Op; id = $Id })
}
function Convert-KfStartType([string]$Mode) {
    switch ($Mode) { 'Auto' { return 'Automatic' }; 'Automatic' { return 'Automatic' }; 'Manual' { return 'Manual' }
        'Disabled' { return 'Disabled' }; 'Boot' { return 'Boot' }; 'System' { return 'System' }; default { throw 'unknown-start-type' } }
}
function Get-KfEntryKey([string]$Kind, $Row) {
    switch ($Kind) {
        'services' { return ([string]$Row.name).ToLowerInvariant() }
        'tasks' { return ([string]($Row.path + $Row.name)).ToLowerInvariant() }
        'run' { return ([string]($Row.key + '|' + $Row.name)).ToLowerInvariant() }
        'startup' { return ([string]$Row.path).ToLowerInvariant() }
    }
    throw 'unknown-entry-kind'
}
function Assert-KfEntries($Entries) {
    Assert-KfFields $Entries @('services','tasks','run','startup')
    foreach ($kind in @('services','tasks','run','startup')) {
        $seen = @{}
        foreach ($r in @($Entries.$kind)) {
            switch ($kind) {
                'services' {
                    Assert-KfFields $r @('name','display_name','binary_path','start_type','account','service_type','dependencies','delayed_auto','status')
                    if ($r.name -notmatch '^[A-Za-z0-9_. -]{1,256}$') { throw 'invalid-service-name' }
                    $null = Convert-KfStartType $r.start_type
                    if ($r.delayed_auto -isnot [bool]) { throw 'invalid-delayed-start' }
                }
                'tasks' {
                    Assert-KfFields $r @('path','name','xml','enabled','status','commands')
                    if ($r.path -notmatch '^\\(?:[A-Za-z0-9_.{}() -]+\\)*$' -or
                        $r.name -notmatch '^[A-Za-z0-9_.{}() -]+$' -or $r.path -match '\\\.\.?\\') { throw 'invalid-task-name' }
                    if ($r.enabled -isnot [bool]) { throw 'invalid-task-enabled' }
                }
                'run' { Assert-KfFields $r @('key','name','value','kind'); Assert-KfRunKey $r.key }
                'startup' {
                    Assert-KfFields $r @('path','sha256','bytes','target')
                    if ($r.sha256 -cnotmatch '^[0-9a-f]{64}$' -or $r.bytes -lt 0) { throw 'invalid-startup-hash' }
                }
            }
            $key = Get-KfEntryKey $kind $r
            if ($seen.ContainsKey($key)) { throw 'duplicate-entry' }
            $seen[$key] = $true
        }
    }
}
function Assert-KfRunKey([string]$Key) {
    if ($Key -notmatch '^(HKLM:|HKU:\\S-1-5-21-[0-9-]+)\\SOFTWARE\\(WOW6432Node\\)?Microsoft\\Windows\\CurrentVersion\\Run(Once)?$') { throw 'invalid-run-key' }
}
function Test-KfProtectedCommand([string]$Command) {
    return $Command -match '(?i)(?:^|[\\ /"''])(?:sshd|qemu-ga|nvdisplay\.container|nvcontainer)\.exe(?:$|[ "''])'
}
function Get-KfProtectedNames($Entries) {
    $protected = @{}
    foreach ($s in @($Entries.services)) {
        if ($s.service_type -match 'Driver' -or (Test-KfProtectedCommand $s.binary_path)) { $protected[$s.name] = $true }
        foreach ($pattern in $script:KfServiceFloor) { if ($s.name -like $pattern) { $protected[$s.name] = $true } }
    }
    do {
        $changed = $false
        foreach ($s in @($Entries.services)) {
            if ($protected.ContainsKey($s.name)) {
                foreach ($dep in @($s.dependencies)) {
                    if (-not $protected.ContainsKey($dep)) { $protected[$dep] = $true; $changed = $true }
                }
            }
        }
    } while ($changed)
    return $protected
}
function Test-KfProtectedEntry([string]$Kind, $Row) {
    if ($Kind -eq 'services') { return $script:KfProtected.ContainsKey($Row.name) -or $Row.service_type -match 'Driver' }
    if ($Kind -eq 'tasks') {
        if ($Row.path -match '^\\Microsoft\\Windows\\(TPM|CertificateServicesClient|Crypto|Windows Defender|TaskScheduler|Plug and Play)(\\|$)' -or
            $Row.name -match '^KF[_-]') { return $true }
        foreach ($c in @($Row.commands)) { if (Test-KfProtectedCommand $c) { return $true } }
        return $false
    }
    if ($Kind -eq 'run') { return (Test-KfProtectedCommand ([string]$Row.value)) }
    return (Test-KfProtectedCommand ([string]$Row.target))
}
function Assert-KfMutable([string]$Kind, $Row) {
    if (Test-KfProtectedEntry $Kind $Row) { throw 'protected-entry' }
}
function Get-KfCatalog {
    if ($null -ne $script:KfFixture) {
        if (@($script:KfFixture.unloaded_profiles).Count -ne 0) { throw 'unloaded-user-profile' }
        Assert-KfEntries $script:KfFixture.entries
        return [pscustomobject]@{ machine_id = $script:KfFixture.machine_id; entries = $script:KfFixture.entries }
    }
    Assert-KfWindows
    $services = @(
        foreach ($s in @(Get-CimInstance Win32_Service) + @(Get-CimInstance Win32_SystemDriver)) {
            $driver = $s.ServiceType -match 'Driver'
            $deps = @(); $delayed = $false
            if (-not $driver) {
                $deps = @((Get-Service -Name $s.Name).RequiredServices | ForEach-Object { $_.Name })
                $p = Get-ItemProperty -LiteralPath ('HKLM:\SYSTEM\CurrentControlSet\Services\' + $s.Name)
                $delayed = (Get-KfProp $p 'DelayedAutoStart' 0) -eq 1
            }
            [pscustomobject]@{ name=[string]$s.Name; display_name=[string]$s.DisplayName; binary_path=[string]$s.PathName
                start_type=(Convert-KfStartType $s.StartMode); account=[string](Get-KfProp $s 'StartName' 'LocalSystem')
                service_type=[string]$s.ServiceType; dependencies=@($deps); delayed_auto=[bool]$delayed; status=[string]$s.State }
        }
    )
    $tasks = @(foreach ($t in Get-ScheduledTask) {
        [pscustomobject]@{ path=[string]$t.TaskPath; name=[string]$t.TaskName
            xml=[string](Export-ScheduledTask -TaskPath $t.TaskPath -TaskName $t.TaskName)
            enabled=[bool]$t.Settings.Enabled; status=[string]$t.State
            commands=@($t.Actions | ForEach-Object { [string](Get-KfProp $_ 'Execute' '') }) }
    })
    $keys = New-Object 'System.Collections.Generic.List[string]'
    $roots = New-Object 'System.Collections.Generic.List[string]'
    $roots.Add([Environment]::GetFolderPath('CommonStartup'))
    $prefixes = New-Object 'System.Collections.Generic.List[string]'
    $prefixes.Add('HKLM:')
    foreach ($p in Get-CimInstance Win32_UserProfile | Where-Object { -not $_.Special }) {
        if (-not $p.Loaded) { throw 'unloaded-user-profile' }
        $prefixes.Add('HKU:\' + $p.SID)
        $hk = [Microsoft.Win32.Registry]::Users.OpenSubKey($p.SID + '\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders')
        try { $root = if ($null -ne $hk) { [string]$hk.GetValue('Startup',$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { '' } }
        finally { if ($null -ne $hk) { $hk.Dispose() } }
        if (-not $root) { $root = '%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup' }
        $root = $root.Replace('%USERPROFILE%', $p.LocalPath).Replace('%APPDATA%', $p.LocalPath + '\AppData\Roaming').Replace('%LOCALAPPDATA%', $p.LocalPath + '\AppData\Local')
        $root = [Environment]::ExpandEnvironmentVariables($root)
        if ($root.StartsWith('\\') -or $root.Contains('%')) { throw 'unresolved-startup-folder' }
        $roots.Add($root)
    }
    foreach ($prefix in $prefixes) { foreach ($wow in @('', 'WOW6432Node\')) { foreach ($leaf in @('Run','RunOnce')) {
        $keys.Add($prefix + '\SOFTWARE\' + $wow + 'Microsoft\Windows\CurrentVersion\' + $leaf)
    } } }
    $run = @(foreach ($key in $keys) {
        $r = Open-KfRunKey $key $false
        if ($null -eq $r) { continue }
        try { foreach ($name in $r.GetValueNames()) { if ($name) {
            [pscustomobject]@{ key=$key; name=[string]$name; kind=[string]$r.GetValueKind($name)
                value=$r.GetValue($name,$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
        } } } finally { $r.Dispose() }
    })
    $startup = @(foreach ($root in @($roots | Select-Object -Unique)) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        $dir = Get-Item -LiteralPath $root
        if ($dir.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'startup-reparse-point' }
        foreach ($f in Get-ChildItem -LiteralPath $root -Force) {
            if ($f.PSIsContainer -or ($f.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'unsupported-startup-item' }
            $target = $f.FullName
            if ($f.Extension -ieq '.lnk') { $shell = New-Object -ComObject WScript.Shell; $target = $shell.CreateShortcut($f.FullName).TargetPath }
            [pscustomobject]@{ path=[string]$f.FullName; sha256=(Get-KfHash $f.FullName); bytes=$f.Length; target=[string]$target }
        }
    })
    $entries = [pscustomobject]@{ services=@($services); tasks=@($tasks); run=@($run); startup=@($startup) }
    Assert-KfEntries $entries
    return [pscustomobject]@{ machine_id=[string](Get-CimInstance Win32_ComputerSystemProduct).UUID; entries=$entries }
}
function Open-KfRunKey([string]$Key, [bool]$Writable) {
    Assert-KfWindows; Assert-KfRunKey $Key
    if ($Key.StartsWith('HKLM:')) { return [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($Key.Substring(6),$Writable) }
    return [Microsoft.Win32.Registry]::Users.OpenSubKey($Key.Substring(5),$Writable)
}
function Get-KfServiceMap($Rows) { $map=@{}; foreach ($r in @($Rows)) { $map[$r.name]=$r }; return $map }
function Get-KfOrder($Rows) {
    $map=Get-KfServiceMap $Rows; $seen=@{}; $visiting=@{}; $order=New-Object 'System.Collections.Generic.List[string]'
    function Visit-KfService([string]$Name) {
        if ($seen.ContainsKey($Name)) { return }
        if ($visiting.ContainsKey($Name)) { throw 'dependency-cycle' }
        $visiting[$Name]=$true
        foreach ($d in @($map[$Name].dependencies)) { if ($map.ContainsKey($d)) { Visit-KfService $d } }
        $visiting.Remove($Name); $seen[$Name]=$true; $order.Add($Name)
    }
    foreach ($n in @($map.Keys | Sort-Object)) { Visit-KfService $n }
    return $order.ToArray()
}
function Set-KfServiceMode($Row, [string]$Mode) {
    Assert-KfMutable 'services' $Row; Add-KfAction 'set_mode' $Row.name
    if ($Mode -notin @('Automatic','Manual','Disabled')) { throw 'unsupported-service-mode' }
    if ($null -ne $script:KfFixture) {
        if (@(Get-KfProp $script:KfFixture 'noop_on' @()) -notcontains ('set_mode:' + $Row.name)) { $Row.start_type=$Mode }
        return
    }
    Assert-KfWindows
    Set-Service -Name $Row.name -StartupType $Mode -ErrorAction Stop
    if ($Mode -eq 'Automatic') { Set-ItemProperty -LiteralPath ('HKLM:\SYSTEM\CurrentControlSet\Services\' + $Row.name) -Name DelayedAutoStart -Type DWord -Value ([int]$Row.delayed_auto) }
}
function Wait-KfService($Row, [string]$Status, [Diagnostics.Stopwatch]$Clock, [int]$TimeoutSec) {
    Add-KfAction 'wait' $Row.name
    if ($null -ne $script:KfFixture) { if ($Row.status -cne $Status) { throw 'service-wait-failed' }; return }
    Assert-KfWindows
    $remaining = $TimeoutSec - $Clock.Elapsed.TotalSeconds
    if ($remaining -le 0) { throw 'service-deadline' }
    (Get-Service -Name $Row.name).WaitForStatus([ServiceProcess.ServiceControllerStatus]::$Status,[TimeSpan]::FromSeconds($remaining))
}
function Stop-KfService($Row, $Clock, [int]$TimeoutSec) {
    Assert-KfMutable 'services' $Row; Add-KfAction 'stop' $Row.name
    if ($null -ne $script:KfFixture) { $Row.status='Stopped' }
    else { Assert-KfWindows; $svc=Get-Service -Name $Row.name; if ($svc.Status -ne 'Stopped') { $svc.Stop() } }
    Wait-KfService $Row 'Stopped' $Clock $TimeoutSec
}
function Start-KfService($Row, $Clock, [int]$TimeoutSec) {
    Assert-KfMutable 'services' $Row; Add-KfAction 'start' $Row.name
    if ($null -ne $script:KfFixture) { $Row.status='Running' }
    else { Assert-KfWindows; $svc=Get-Service -Name $Row.name; if ($svc.Status -ne 'Running') { $svc.Start() } }
    Wait-KfService $Row 'Running' $Clock $TimeoutSec
}
function Disable-KfTask($Row) {
    Assert-KfMutable 'tasks' $Row; Add-KfAction 'disable_task' (Get-KfEntryKey 'tasks' $Row)
    if ($null -ne $script:KfFixture) {
        if (@(Get-KfProp $script:KfFixture 'noop_on' @()) -notcontains ('disable_task:' + (Get-KfEntryKey 'tasks' $Row))) {
            $Row.enabled=$false; $Row.status='Ready'
        }
        return
    }
    Assert-KfWindows
    Disable-ScheduledTask -TaskPath $Row.path -TaskName $Row.name | Out-Null
    if ($Row.status -in @('Running','Queued')) { Stop-ScheduledTask -TaskPath $Row.path -TaskName $Row.name }
}
function Remove-KfRun($Row) {
    Assert-KfMutable 'run' $Row; Add-KfAction 'remove_run' (Get-KfEntryKey 'run' $Row)
    if ($null -ne $script:KfFixture) { $script:KfFixture.entries.run=@($script:KfFixture.entries.run | Where-Object { (Get-KfEntryKey 'run' $_) -ne (Get-KfEntryKey 'run' $Row) }); return }
    Assert-KfWindows; $key=Open-KfRunKey $Row.key $true
    try { $key.DeleteValue($Row.name,$true) } finally { $key.Dispose() }
}
function Remove-KfStartup($Row, [string]$QuarantineDir) {
    Assert-KfMutable 'startup' $Row; Add-KfAction 'quarantine_startup' $Row.path
    if ($null -ne $script:KfFixture) {
        $script:KfActions[$script:KfActions.Count-1]['quarantine_path']=$QuarantineDir + '/fixture-' + $Row.sha256
        $script:KfFixture.entries.startup=@($script:KfFixture.entries.startup | Where-Object { $_.path -ne $Row.path }); return
    }
    Assert-KfWindows
    if ((Get-KfHash $Row.path) -cne $Row.sha256) { throw 'startup-item-changed' }
    New-Item -ItemType Directory -Force -Path $QuarantineDir | Out-Null
    $dest=Join-Path $QuarantineDir ([guid]::NewGuid().ToString('N') + '-' + [IO.Path]::GetFileName($Row.path))
    $script:KfActions[$script:KfActions.Count-1]['quarantine_path']=$dest
    Move-Item -LiteralPath $Row.path -Destination $dest
}
function Read-KfFootprint([string]$Path) {
    $f=Read-KfJson $Path
    Assert-KfFields $f @('schema_version','kind','machine_id','created_utc','app_id','installer_sha256','before_sha256','after_sha256','entries','changed_existing')
    Assert-KfVersion $f 'app_footprint'; Assert-KfId $f.app_id; Assert-KfEntries $f.entries
    foreach ($h in @($f.installer_sha256,$f.before_sha256,$f.after_sha256)) { if ($h -cnotmatch '^[0-9a-f]{64}$') { throw 'invalid-sha256' } }
    return $f
}
function Assert-KfServiceIdentity($Current, $Recorded) {
    foreach ($k in @('name','binary_path','account','service_type')) { if ($Current.$k -cne $Recorded.$k) { throw 'service-identity-changed' } }
    if ((@($Current.dependencies | Sort-Object) -join '|') -cne (@($Recorded.dependencies | Sort-Object) -join '|')) { throw 'service-dependencies-changed' }
}
function Assert-KfQuiet($Catalog, [string[]]$AllowedServices=@(), [string[]]$AllowedTasks=@()) {
    foreach ($s in @($Catalog.entries.services)) {
        if (-not (Test-KfProtectedEntry 'services' $s) -and $AllowedServices -notcontains $s.name) {
            if ($s.start_type -ne 'Disabled' -or $s.status -ne 'Stopped') { throw 'service-not-quiescent' }
        }
    }
    foreach ($t in @($Catalog.entries.tasks)) {
        if (-not (Test-KfProtectedEntry 'tasks' $t) -and $AllowedTasks -notcontains (Get-KfEntryKey 'tasks' $t)) {
            if ($t.enabled -or $t.status -in @('Running','Queued')) { throw 'task-not-quiescent' }
        }
    }
    foreach ($kind in @('run','startup')) { foreach ($r in @($Catalog.entries.$kind)) {
        if (-not (Test-KfProtectedEntry $kind $r)) { throw 'startup-not-quiescent' }
    } }
}
function Reset-KfLab($Catalog, [string]$QuarantineDir, $Clock, [int]$TimeoutSec) {
    $rows=@($Catalog.entries.services | Where-Object { -not (Test-KfProtectedEntry 'services' $_) })
    $order=@(Get-KfOrder $rows); $map=Get-KfServiceMap $rows
    foreach ($s in $rows) { if ($s.start_type -ne 'Disabled') { Set-KfServiceMode $s 'Disabled' } }
    for ($i=$order.Count-1; $i -ge 0; $i--) {
        if ($map[$order[$i]].status -ne 'Stopped') { Stop-KfService $map[$order[$i]] $Clock $TimeoutSec }
    }
    foreach ($t in @($Catalog.entries.tasks)) {
        if (-not (Test-KfProtectedEntry 'tasks' $t) -and ($t.enabled -or $t.status -in @('Running','Queued'))) { Disable-KfTask $t }
    }
    foreach ($r in @($Catalog.entries.run)) { if (-not (Test-KfProtectedEntry 'run' $r)) { Remove-KfRun $r } }
    foreach ($s in @($Catalog.entries.startup)) { if (-not (Test-KfProtectedEntry 'startup' $s)) { Remove-KfStartup $s $QuarantineDir } }
    Assert-KfQuiet (Get-KfCatalog)
}
function Get-KfManualTaskXml([string]$Text) {
    $settings=New-Object Xml.XmlReaderSettings
    $settings.DtdProcessing=[Xml.DtdProcessing]::Prohibit; $settings.XmlResolver=$null
    $reader=[Xml.XmlReader]::Create((New-Object IO.StringReader($Text)),$settings)
    try { $doc=New-Object Xml.XmlDocument; $doc.XmlResolver=$null; $doc.Load($reader) } finally { $reader.Dispose() }
    $ns=New-Object Xml.XmlNamespaceManager($doc.NameTable); $uri='http://schemas.microsoft.com/windows/2004/02/mit/task'; $ns.AddNamespace('t',$uri)
    $logon=$doc.SelectSingleNode('/t:Task/t:Principals/t:Principal/t:LogonType',$ns)
    if ($null -eq $logon -or $logon.InnerText -notin @('ServiceAccount','InteractiveToken','S4U')) { throw 'task-needs-credentials' }
    $triggers=$doc.SelectSingleNode('/t:Task/t:Triggers',$ns)
    if ($null -ne $triggers) { $triggers.RemoveAll() }
    $enabled=$doc.SelectSingleNode('/t:Task/t:Settings/t:Enabled',$ns)
    if ($null -eq $enabled) { throw 'unsupported-task-settings' }
    $enabled.InnerText='true'
    return $doc.OuterXml
}
function Import-KfManualTask($Current, $Original) {
    Assert-KfMutable 'tasks' $Current
    $xml=Get-KfManualTaskXml $Original.xml
    Add-KfAction 'import_task' (Get-KfEntryKey 'tasks' $Original)
    if ($null -ne $script:KfFixture) { $Current.xml=$xml; $Current.enabled=$true; $Current.status='Ready'; return }
    Assert-KfWindows
    Register-ScheduledTask -TaskPath $Original.path -TaskName $Original.name -Xml $xml -Force | Out-Null
}
