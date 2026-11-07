# box_facts.ps1 -- read-only facts of the box (no secrets, no scores). Prints BOXFACT lines.
$ErrorActionPreference = 'Continue'
$smi = "$env:windir\System32\nvidia-smi.exe"
$os = Get-CimInstance Win32_OperatingSystem
"BOXFACT os='$($os.Caption)' build=$($os.BuildNumber)"
"BOXFACT cpu='$((Get-CimInstance Win32_Processor | Select-Object -First 1).Name)' logical=$((Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors) ram_gb=$([math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1))"
"BOXFACT system='$((Get-CimInstance Win32_ComputerSystem).Manufacturer) $((Get-CimInstance Win32_ComputerSystem).Model)'"
Get-CimInstance Win32_VideoController | ForEach-Object { "BOXFACT video name='$($_.Name)' driver=$($_.DriverVersion) code=$($_.ConfigManagerErrorCode) res=$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution) vram=$($_.AdapterRAM)" }
"BOXFACT disk C free_gb=$([math]::Round((Get-PSDrive C).Free/1GB,1)) size_gb=$([math]::Round((Get-Volume -DriveLetter C).Size/1GB,1))"
& $smi --query-gpu=name,driver_version,memory.total,pcie.link.gen.current,pcie.link.width.current,vbios_version --format=csv,noheader | ForEach-Object { "BOXFACT smi $_" }
& $smi -q -d MEMORY | ForEach-Object { "BOXFACT smiq $_" }
Get-PnpDevice -PresentOnly -Class Display | ForEach-Object { "BOXFACT pnp $($_.FriendlyName) status=$($_.Status)" }
