# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# display_probe.ps1 -- read-only: what does Windows believe about displays/monitors? One line per fact: `DSP <what> <value>`.
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class KfDsp {
  [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint flags, out uint paths, out uint modes);
  public static string Counts() {
    string s = "";
    uint[] fl = new uint[] { 1, 2, 4 }; string[] nm = new string[] { "ALL_PATHS", "ONLY_ACTIVE_PATHS", "DATABASE_CURRENT" };
    for (int i = 0; i < fl.Length; i++) { uint p, m; int r = GetDisplayConfigBufferSizes(fl[i], out p, out m); s += nm[i] + ": rc=" + r + " paths=" + p + " modes=" + m + "; "; }
    return s;
  }
}
'@
"DSP session " + [System.Diagnostics.Process]::GetCurrentProcess().SessionId + " user " + (whoami)
try { "DSP QueryDisplayConfig " + [KfDsp]::Counts() } catch { "DSP QDC exception " + $_.Exception.Message }
Get-CimInstance Win32_VideoController | ForEach-Object {
  "DSP VideoController Name='$($_.Name)' Status=$($_.Status) ConfigManagerErrorCode=$($_.ConfigManagerErrorCode) H=$($_.CurrentHorizontalResolution) V=$($_.CurrentVerticalResolution) Hz=$($_.CurrentRefreshRate) VideoProcessor='$($_.VideoProcessor)' PNP=$($_.PNPDeviceID)"
}
$m = @(Get-CimInstance Win32_DesktopMonitor -ErrorAction SilentlyContinue)
"DSP Win32_DesktopMonitor count=" + $m.Count
$m | ForEach-Object { "DSP DesktopMonitor Name='$($_.Name)' Status=$($_.Status) PNP=$($_.PNPDeviceID) H=$($_.ScreenWidth) V=$($_.ScreenHeight)" }
$ids = @(Get-CimInstance -Namespace root\wmi WmiMonitorID -ErrorAction SilentlyContinue)
"DSP WmiMonitorID count=" + $ids.Count
$ids | ForEach-Object { "DSP WmiMonitorID Instance=$($_.InstanceName) Active=$($_.Active) Product=$($_.ProductCodeID) Mfg=$($_.ManufacturerName -join ',')" }
$bp = @(Get-CimInstance -Namespace root\wmi WmiMonitorBasicDisplayParams -ErrorAction SilentlyContinue)
"DSP WmiMonitorBasicDisplayParams count=" + $bp.Count
$bp | ForEach-Object { "DSP BasicDisplayParams Instance=$($_.InstanceName) Active=$($_.Active) MaxH=$($_.MaxHorizontalImageSize)cm MaxV=$($_.MaxVerticalImageSize)cm Digital=$($_.VideoInputType)" }
$sm = @(Get-CimInstance -Namespace root\wmi WmiMonitorListedSupportedSourceModes -ErrorAction SilentlyContinue)
"DSP WmiMonitorListedSupportedSourceModes count=" + $sm.Count
$sm | ForEach-Object { "DSP ListedModes Instance=$($_.InstanceName) n=$($_.NumOfMonitorSourceModes)" }
$pd = @(Get-PnpDevice -Class Monitor -ErrorAction SilentlyContinue)
"DSP PnpDevice Class=Monitor count=" + $pd.Count
$pd | ForEach-Object { "DSP PnpMonitor '$($_.FriendlyName)' Status=$($_.Status) Present=$($_.Present) Id=$($_.InstanceId)" }
Get-PnpDevice -Class Display -ErrorAction SilentlyContinue | ForEach-Object { "DSP PnpDisplay '$($_.FriendlyName)' Status=$($_.Status) Present=$($_.Present) Problem=$($_.Problem) Id=$($_.InstanceId)" }
