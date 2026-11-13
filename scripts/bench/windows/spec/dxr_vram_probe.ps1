# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxr_vram_probe.ps1 -- what a guest application can read about the GPU: DXGI adapter descriptions (dedicated video memory),
# D3D12 feature support (OPTIONS5 RaytracingTier, OPTIONS12 enhanced barriers is not asked), system RAM, nvidia-smi memory.
# STATUS: CODE, 2026-11-12 (claude/spec-pc-20261112), a diagnostic for the enscape-01 refusal; read-only, prints no score.
$src = @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class P {
  [DllImport("dxgi.dll")] static extern int CreateDXGIFactory1(ref Guid iid, out IntPtr f);
  [DllImport("d3d12.dll")] static extern int D3D12CreateDevice(IntPtr adapter, int fl, ref Guid iid, out IntPtr dev);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int EnumA(IntPtr self, uint i, out IntPtr a);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int GetDesc1(IntPtr self, IntPtr desc);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int CheckFS(IntPtr self, int feature, IntPtr data, int size);
  static IntPtr Vt(IntPtr o, int i) { return Marshal.ReadIntPtr(Marshal.ReadIntPtr(o), i * IntPtr.Size); }
  public static string Run() {
    var sb = new StringBuilder();
    Guid f1 = new Guid("770aae78-f26f-4dba-a829-253c83d1b387"), d12 = new Guid("189819f1-1db6-4b57-be54-1821339b85f7");
    IntPtr fac; int hr = CreateDXGIFactory1(ref f1, out fac);
    if (hr != 0) return "PROBE dxgi_factory hr=0x" + hr.ToString("x");
    var en = (EnumA)Marshal.GetDelegateForFunctionPointer(Vt(fac, 12), typeof(EnumA));
    for (uint i = 0; ; i++) {
      IntPtr ad; if (en(fac, i, out ad) != 0) break;
      IntPtr buf = Marshal.AllocHGlobal(512);
      var gd = (GetDesc1)Marshal.GetDelegateForFunctionPointer(Vt(ad, 10), typeof(GetDesc1));
      gd(ad, buf);
      string name = Marshal.PtrToStringUni(buf, 128).Split('\0')[0];
      long ded = Marshal.ReadInt64(buf, 256 + 16), dsys = Marshal.ReadInt64(buf, 256 + 24), shr = Marshal.ReadInt64(buf, 256 + 32);
      uint ven = (uint)Marshal.ReadInt32(buf, 256);
      sb.AppendFormat("PROBE adapter{0} name='{1}' vendor=0x{2:x} dedicated_video_mb={3} dedicated_sys_mb={4} shared_sys_mb={5}\n", i, name, ven, ded >> 20, dsys >> 20, shr >> 20);
      IntPtr dev; int h = D3D12CreateDevice(ad, 0xb000, ref d12, out dev);
      if (h != 0) { sb.AppendFormat("PROBE adapter{0} d3d12_create hr=0x{1:x}\n", i, h); continue; }
      var cf = (CheckFS)Marshal.GetDelegateForFunctionPointer(Vt(dev, 13), typeof(CheckFS));
      IntPtr o5 = Marshal.AllocHGlobal(16); Marshal.WriteInt32(o5, 0, 0); Marshal.WriteInt32(o5, 4, 0); Marshal.WriteInt32(o5, 8, 0);
      int r5 = cf(dev, 27, o5, 12);
      sb.AppendFormat("PROBE adapter{0} d3d12 OPTIONS5 hr=0x{1:x} RaytracingTier={2} (0=none 10=1.0 11=1.1)\n", i, r5, Marshal.ReadInt32(o5, 8));
      IntPtr o0 = Marshal.AllocHGlobal(64); for (int k = 0; k < 64; k += 4) Marshal.WriteInt32(o0, k, 0);
      int r0 = cf(dev, 0, o0, 44);
      sb.AppendFormat("PROBE adapter{0} d3d12 OPTIONS hr=0x{1:x} ResourceBindingTier={2} TiledResourcesTier={3}\n", i, r0, Marshal.ReadInt32(o0, 16), Marshal.ReadInt32(o0, 12));
      IntPtr o7 = Marshal.AllocHGlobal(16); for (int k = 0; k < 16; k += 4) Marshal.WriteInt32(o7, k, 0);
      int r7 = cf(dev, 32, o7, 8);   // OPTIONS7: MeshShaderTier, SamplerFeedbackTier
      sb.AppendFormat("PROBE adapter{0} d3d12 OPTIONS7 hr=0x{1:x} MeshShaderTier={2} SamplerFeedbackTier={3}\n", i, r7, Marshal.ReadInt32(o7, 0), Marshal.ReadInt32(o7, 4));
    }
    return sb.ToString();
  }
}
'@
Add-Type -TypeDefinition $src -Language CSharp
Write-Output ("PROBE_TIME " + (Get-Date).ToUniversalTime().ToString('o'))
Write-Output ([P]::Run())
$cs = Get-CimInstance Win32_ComputerSystem
Write-Output ("PROBE system_ram_mb=" + [math]::Round($cs.TotalPhysicalMemory / 1MB))
Get-CimInstance Win32_VideoController | ForEach-Object { Write-Output ("PROBE wmi_video name='{0}' driver={1} adapter_ram_bytes={2}" -f $_.Name, $_.DriverVersion, $_.AdapterRAM) }
$smi = Join-Path $env:SystemRoot 'System32\nvidia-smi.exe'
if (Test-Path $smi) { & $smi --query-gpu=name,driver_version,memory.total,memory.used,memory.free --format=csv,noheader 2>&1 | ForEach-Object { Write-Output "PROBE smi $_" } }
