# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_guest_setup.ps1 -- once per fresh guest, as SYSTEM through QGA: find the app disk (label KFAPPS),
# check its identity, create the working directories, enumerate the DXGI adapters (C:\kf\adapters.json,
# the LUID of the NVIDIA adapter is what the GPU-use proof compares against), make the session usable
# for unattended runs (no sleep, no screen saver, Defender exclusions) and print one `KFSETUP {json}` line.
param([string]$ExpectedIdentity = '')
$ErrorActionPreference = 'Continue'
. "$PSScriptRoot\kf_common.ps1"
Initialize-KfDirs
$info = @{ ok = $false; notes = @() }
$cd = Get-AppDiskDrive
if (-not $cd) {
    # the hot-plugged disk can take a few seconds to appear
    for ($i = 0; $i -lt 30 -and -not $cd; $i++) { Start-Sleep -Seconds 2; $cd = Get-AppDiskDrive }
}
$info.cd = $cd
if ($cd) {
    $idf = Join-Path $cd 'KFAPPS.ID'
    if (Test-Path $idf) { $info.image_identity = (Get-Content -Raw $idf).Trim() }
    if ($ExpectedIdentity -and $info.image_identity -ne $ExpectedIdentity) { $info.notes += 'image identity differs from the manifest' }
    Set-Content -Path 'C:\kf\cd.txt' -Value $cd -Encoding ASCII
    if (Test-Path (Join-Path $cd 'tools')) {
        New-Item -ItemType Directory -Force -Path 'C:\kfapps\tools' | Out-Null
        Copy-Item -Force -Path (Join-Path $cd 'tools\*.exe') -Destination 'C:\kfapps\tools'
        $info.tools = @(Get-ChildItem 'C:\kfapps\tools' -Filter *.exe | ForEach-Object { $_.Name })
    }
}

# DXGI adapter list (the same struct walk as d3d12_signal_probe.ps1; WARP is vendor 0x1414)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public static class KfDxgi {
  [DllImport("dxgi.dll")] static extern int CreateDXGIFactory1(ref Guid riid, out IntPtr ppv);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnEnum(IntPtr self, uint i, out IntPtr a);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnDesc1(IntPtr self, byte[] desc);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate uint FnRelease(IntPtr self);
  static IntPtr Slot(IntPtr obj, int idx) { IntPtr vt = Marshal.ReadIntPtr(obj); return Marshal.ReadIntPtr(vt, idx * IntPtr.Size); }
  static T D<T>(IntPtr obj, int idx) { return (T)(object)Marshal.GetDelegateForFunctionPointer(Slot(obj, idx), typeof(T)); }
  public static string[] List() {
    var res = new List<string>();
    Guid fac = new Guid("770aae78-f26f-4dba-a829-253c83d1b387");
    IntPtr f; int hr = CreateDXGIFactory1(ref fac, out f);
    if (hr != 0) { res.Add("ERR " + hr.ToString("x8")); return res.ToArray(); }
    for (uint i = 0; i < 16; i++) {
      IntPtr a; hr = D<FnEnum>(f, 12)(f, i, out a); if (hr != 0) break;
      byte[] d = new byte[312]; D<FnDesc1>(a, 10)(a, d);
      string name = System.Text.Encoding.Unicode.GetString(d, 0, 256).Split('\0')[0];
      uint ven = BitConverter.ToUInt32(d, 256), dev = BitConverter.ToUInt32(d, 260);
      ulong ded = BitConverter.ToUInt64(d, 272);
      uint lo = BitConverter.ToUInt32(d, 296); int hi = BitConverter.ToInt32(d, 300);
      uint flags = BitConverter.ToUInt32(d, 304);
      res.Add(i + "|" + ven + "|" + dev + "|" + hi.ToString("x8") + "|" + lo.ToString("x8") + "|" + (ded >> 20) + "|" + flags + "|" + name);
      D<FnRelease>(a, 2)(a);
    }
    return res.ToArray();
  }
}
'@
$ad = @()
try {
    foreach ($l in [KfDxgi]::List()) {
        $p = $l.Split('|')
        if ($p.Count -ge 8) { $ad += @{ index = [int]$p[0]; vendor = [int]$p[1]; device = [int]$p[2]; luid_high = $p[3]; luid_low = $p[4]; dedicated_mb = [int64]$p[5]; flags = [int]$p[6]; name = $p[7] } }
    }
} catch { $info.notes += "dxgi enumeration failed: $_" }
Write-JsonAtomic @{ adapters = $ad; utc = (Get-Date).ToUniversalTime().ToString('o') } 'C:\kf\adapters.json'
$info.adapters = $ad
$info.nvidia_adapters = @($ad | Where-Object { $_.vendor -eq 4318 }).Count

# unattended-run hygiene (all best effort; failures are recorded, never fatal)
try { powercfg /change standby-timeout-ac 0 | Out-Null; powercfg /change monitor-timeout-ac 0 | Out-Null; powercfg /change hibernate-timeout-ac 0 | Out-Null } catch { $info.notes += 'powercfg failed' }
try { Set-ItemProperty 'HKCU:\Control Panel\Desktop' ScreenSaveActive 0 -ErrorAction SilentlyContinue } catch { }
try { Add-MpPreference -ExclusionPath 'C:\kfapps', 'C:\kf' -ErrorAction Stop } catch { $info.notes += 'Defender exclusion failed (Tamper Protection?)' }
try { Set-MpPreference -DisableRealtimeMonitoring $true -ErrorAction Stop } catch { $info.notes += 'Defender real-time off failed' }

$os = Get-CimInstance Win32_OperatingSystem
$info.os = ('{0} build {1}' -f $os.Caption, $os.BuildNumber)
$info.boot_utc = Get-BootTimeUtc
$info.user = (Get-CimInstance Win32_ComputerSystem).UserName
$info.explorer = [bool](Get-Process explorer -ErrorAction SilentlyContinue)
$smi = Get-SmiPath
if ($smi) {
    $info.smi = (& $smi --query-gpu=name,driver_version,memory.total --format=csv,noheader 2>&1 | Select-Object -First 1) -as [string]
}
$vc = Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion, Status, ConfigManagerErrorCode
$info.video = @($vc | ForEach-Object { '{0} | {1} | {2} | code {3}' -f $_.Name, $_.DriverVersion, $_.Status, $_.ConfigManagerErrorCode })
$info.ok = [bool]($cd -and $info.nvidia_adapters -ge 1)
Write-Output ('KFSETUP ' + (ConvertTo-Json -InputObject $info -Depth 6 -Compress))
if ($info.ok) { exit 0 } else { exit 3 }
