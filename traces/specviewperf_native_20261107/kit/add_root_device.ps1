# add_root_device.ps1 -Inf PATH -HardwareId Root\MttVDD -- create a root-enumerated device node and bind an INF to it (what `devcon install` does; this
# Windows build's pnputil has no /add-device). Used only to attach a virtual display (indirect display driver) to a headless GPU box.
param([Parameter(Mandatory = $true)][string]$Inf, [Parameter(Mandatory = $true)][string]$HardwareId)
$src = @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class RootDev {
  [StructLayout(LayoutKind.Sequential)] public struct SP_DEVINFO_DATA { public int cbSize; public Guid ClassGuid; public int DevInst; public IntPtr Reserved; }
  [DllImport("setupapi.dll", SetLastError=true)] static extern IntPtr SetupDiCreateDeviceInfoList(ref Guid g, IntPtr h);
  [DllImport("setupapi.dll", SetLastError=true, CharSet=CharSet.Unicode)] static extern bool SetupDiCreateDeviceInfoW(IntPtr s, string name, ref Guid g, string desc, IntPtr h, int flags, ref SP_DEVINFO_DATA d);
  [DllImport("setupapi.dll", SetLastError=true, CharSet=CharSet.Unicode)] static extern bool SetupDiSetDeviceRegistryPropertyW(IntPtr s, ref SP_DEVINFO_DATA d, int prop, byte[] buf, int size);
  [DllImport("setupapi.dll", SetLastError=true)] static extern bool SetupDiCallClassInstaller(int fn, IntPtr s, ref SP_DEVINFO_DATA d);
  [DllImport("setupapi.dll", SetLastError=true)] static extern bool SetupDiDestroyDeviceInfoList(IntPtr s);
  [DllImport("newdev.dll", SetLastError=true, CharSet=CharSet.Unicode)] static extern bool UpdateDriverForPlugAndPlayDevicesW(IntPtr h, string hwid, string inf, int flags, out bool reboot);
  public static string Run(string inf, string hwid) {
    Guid cls = new Guid("4D36E968-E325-11CE-BFC1-08002BE10318");
    IntPtr s = SetupDiCreateDeviceInfoList(ref cls, IntPtr.Zero);
    if (s == (IntPtr)(-1)) return "CreateDeviceInfoList failed " + Marshal.GetLastWin32Error();
    SP_DEVINFO_DATA d = new SP_DEVINFO_DATA(); d.cbSize = Marshal.SizeOf(typeof(SP_DEVINFO_DATA));
    if (!SetupDiCreateDeviceInfoW(s, "Display", ref cls, null, IntPtr.Zero, 1, ref d)) return "CreateDeviceInfo failed " + Marshal.GetLastWin32Error();
    byte[] hw = Encoding.Unicode.GetBytes(hwid + "\0\0");
    if (!SetupDiSetDeviceRegistryPropertyW(s, ref d, 1, hw, hw.Length)) return "SetHardwareId failed " + Marshal.GetLastWin32Error();
    if (!SetupDiCallClassInstaller(0x19, s, ref d)) return "DIF_REGISTERDEVICE failed " + Marshal.GetLastWin32Error();
    SetupDiDestroyDeviceInfoList(s);
    bool reboot;
    if (!UpdateDriverForPlugAndPlayDevicesW(IntPtr.Zero, hwid, inf, 1, out reboot)) return "UpdateDriver failed " + Marshal.GetLastWin32Error();
    return "OK reboot=" + reboot;
  }
}
'@
Add-Type -TypeDefinition $src
[RootDev]::Run($Inf, $HardwareId)
