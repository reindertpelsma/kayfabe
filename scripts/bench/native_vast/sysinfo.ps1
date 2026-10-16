# sysinfo.ps1 OUTFILE : system state that could matter (CPU, RAM, OS, power plan, HAGS, Game Mode, displays, GPU, SM count)
param([string]$Out = 'C:\bench\sysinfo.txt')
$ErrorActionPreference = 'Continue'
$o = New-Object System.Collections.Generic.List[string]
function A($s) { $o.Add([string]$s) }
A "date_utc=$((Get-Date).ToUniversalTime().ToString('o'))"
$os = Get-CimInstance Win32_OperatingSystem; A "os=$($os.Caption) build=$($os.BuildNumber) ver=$($os.Version)"
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1; A "cpu=$($cpu.Name) cores=$($cpu.NumberOfCores) logical=$($cpu.NumberOfLogicalProcessors) mhz=$($cpu.MaxClockSpeed)"
$cs = Get-CimInstance Win32_ComputerSystem; A "ram_gb=$([math]::Round($cs.TotalPhysicalMemory/1GB,1)) model=$($cs.Model) manufacturer=$($cs.Manufacturer) hypervisor_present=$($cs.HypervisorPresent)"
A ("powercfg=" + ((powercfg /getactivescheme) -join ' '))
foreach ($k in @('HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers')) { $v = (Get-ItemProperty $k -ErrorAction SilentlyContinue).HwSchMode; A "HwSchMode=$v (2=on, 1=off, absent=default)" }
$gm = Get-ItemProperty 'HKCU:\Software\Microsoft\GameBar' -ErrorAction SilentlyContinue; A "GameBar AutoGameModeEnabled=$($gm.AutoGameModeEnabled) AllowAutoGameMode=$($gm.AllowAutoGameMode)"
A ("UserGpuPreferences=" + ((Get-ItemProperty 'HKCU:\Software\Microsoft\DirectX\UserGpuPreferences' -ErrorAction SilentlyContinue | Out-String).Trim()))
Get-CimInstance Win32_VideoController | ForEach-Object { A "video=$($_.Name) driver=$($_.DriverVersion) status=$($_.Status) mode=$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution)@$($_.CurrentRefreshRate)Hz pnp=$($_.PNPDeviceID)" }
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.Screen]::AllScreens | ForEach-Object { A "screen=$($_.DeviceName) primary=$($_.Primary) bounds=$($_.Bounds)" }
Add-Type -Namespace K -Name Cu -MemberDefinition @'
[DllImport("nvcuda.dll")] public static extern int cuInit(uint f);
[DllImport("nvcuda.dll")] public static extern int cuDeviceGet(out int d, int i);
[DllImport("nvcuda.dll")] public static extern int cuDeviceGetAttribute(out int v, int a, int d);
'@ -ErrorAction SilentlyContinue
try { $r = [K.Cu]::cuInit(0); $d = 0; $r2 = [K.Cu]::cuDeviceGet([ref]$d, 0)
  foreach ($p in @(@('sm_count',16),@('clock_khz',13),@('mem_clock_khz',36),@('bus_width_bits',37),@('l2_bytes',38),@('cc_major',75),@('cc_minor',76))) { $v = 0; $null = [K.Cu]::cuDeviceGetAttribute([ref]$v, $p[1], $d); A "cuda $($p[0])=$v" }
} catch { A "cuda attr error: $_" }
A '--- nvidia-smi'
$smi = (& nvidia-smi 2>&1); $smi | ForEach-Object { A $_ }
A '--- nvidia-smi --query-gpu'
(& nvidia-smi --query-gpu=name,driver_version,vbios_version,pci.bus_id,pci.device_id,pstate,pcie.link.gen.current,pcie.link.gen.max,pcie.link.width.current,pcie.link.width.max,clocks.sm,clocks.mem,clocks.max.sm,clocks.max.mem,power.limit,power.default_limit,power.max_limit,memory.total,driver_model.current --format=csv) | ForEach-Object { A $_ }
$o | Set-Content -Path $Out -Encoding UTF8
