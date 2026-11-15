# guest-side read-only collection (own authored script); prints text only
$ErrorActionPreference='SilentlyContinue'
"== time " + (Get-Date).ToUniversalTime().ToString('o')
"== Win32_Processor"
Get-CimInstance Win32_Processor | Select-Object Manufacturer,Name,Description,Family,Architecture,ProcessorId,NumberOfCores,NumberOfLogicalProcessors,MaxClockSpeed,L2CacheSize,L3CacheSize,VirtualizationFirmwareEnabled,SecondLevelAddressTranslationExtensions | Format-List | Out-String -Width 200
"== registry CentralProcessor\0"
Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0' | Select-Object VendorIdentifier,Identifier,ProcessorNameString,'~MHz',FeatureSet,'Update Revision','Platform Specific Field1' | Format-List | Out-String -Width 200
"== systeminfo (processor, hypervisor lines)"
systeminfo | Select-String -Pattern 'Processor|Hyper-V|Virtualization|Hypervisor|System Boot Time|OS Name|OS Version' | ForEach-Object { $_.Line }
"== cpuid probe"
if (Test-Path C:\kfhw\cpuid_probe.exe) { & C:\kfhw\cpuid_probe.exe }
"== IsProcessorFeaturePresent"
Add-Type -Namespace K -Name P2 -MemberDefinition '[DllImport("kernel32.dll")] public static extern bool IsProcessorFeaturePresent(uint f);'
foreach($i in 0,1,2,3,6,7,8,9,10,12,13,14,17,18,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,36,37,38,39,40,41,43,44,45){ "pf{0}={1}" -f $i,[K.P2]::IsProcessorFeaturePresent($i) }
"== System events of interest (since boot)"
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
Get-WinEvent -FilterHashtable @{LogName='System';StartTime=$boot} | Where-Object { $_.LevelDisplayName -in 'Critical','Error','Warning' -or $_.ProviderName -match 'WHEA|Hyper-V|Hypervisor|Processor|Kernel-Power|BugCheck|WER-SystemErrorReporting|Microsoft-Windows-Kernel-Boot|Licens|Security-SPP|wlms' } | Sort-Object TimeCreated | ForEach-Object { '{0} {1} id={2} {3}' -f $_.TimeCreated.ToUniversalTime().ToString('s'),$_.ProviderName,$_.Id,(($_.Message -split "`n")[0]) } | Select-Object -First 120
"== previous-boot crash evidence"
Get-WinEvent -FilterHashtable @{LogName='System';Id=41,1001,6008} -MaxEvents 20 | ForEach-Object { '{0} {1} id={2} {3}' -f $_.TimeCreated.ToUniversalTime().ToString('s'),$_.ProviderName,$_.Id,(($_.Message -split "`n")[0]) }
Get-ChildItem C:\Windows\Minidump,C:\Windows\MEMORY.DMP | ForEach-Object { $_.FullName + ' ' + $_.LastWriteTime }
"== Windows licensing state"
Get-CimInstance SoftwareLicensingProduct | Where-Object { $_.PartialProductKey } | Select-Object Name,LicenseStatus,GracePeriodRemaining | Format-List | Out-String
"== Application log (SPP / activation / app licence providers)"
Get-WinEvent -FilterHashtable @{LogName='Application';StartTime=$boot} | Where-Object { $_.ProviderName -match 'Licens|Software Protection|SolidWorks|3DMark|UL|Flexnet|FNP|WER' -or $_.Id -eq 1001 } | Sort-Object TimeCreated | ForEach-Object { '{0} {1} id={2} {3}' -f $_.TimeCreated.ToUniversalTime().ToString('s'),$_.ProviderName,$_.Id,(($_.Message -split "`n")[0]) } | Select-Object -First 60
