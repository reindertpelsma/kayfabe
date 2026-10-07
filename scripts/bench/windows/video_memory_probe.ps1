# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# video_memory_probe.ps1 -- read-only measurement of what the Windows graphics stack believes about the NVIDIA
# adapter's video memory (run inside the guest as SYSTEM through QGA; no compiler needed: Add-Type + P/Invoke).
# One line per measurement: `VMP <what> <value>`. Nothing is written except to stdout.
#   1. DXGI adapter descriptions (GetDesc1, GetDesc2): DedicatedVideoMemory, DedicatedSystemMemory,
#      SharedSystemMemory, LUID, flags -- every adapter.
#   2. IDXGIAdapter3::QueryVideoMemoryInfo: Budget, CurrentUsage, AvailableForReservation, CurrentReservation
#      for the LOCAL and NON_LOCAL segment groups.
#   3. D3DKMT (gdi32) on the same LUID: KMTQAITYPE_GETSEGMENTSIZE (what dxgkrnl's segment manager holds),
#      DRIVERVERSION, ADAPTERTYPE; then D3DKMTCreateDevice / D3DKMTDestroyDevice (does a kernel WDDM device
#      exist? the D3D runtime's failure would then be after this step, in the user-mode driver or later).
#   4. D3D11/D3D12 CreateDevice per feature level on the NVIDIA adapter, and on the Basic Render Driver (control).
#   5. Win32_VideoController AdapterRAM and the display class key's HardwareInformation.* values.
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KfVmp {
  [DllImport("dxgi.dll")] static extern int CreateDXGIFactory1(ref Guid riid, out IntPtr ppv);
  [DllImport("d3d12.dll")] static extern int D3D12CreateDevice(IntPtr adapter, int level, ref Guid riid, out IntPtr dev);
  [DllImport("d3d11.dll")] static extern int D3D11CreateDevice(IntPtr adapter, int driverType, IntPtr sw, uint flags, IntPtr levels, uint nlevels, uint sdk, out IntPtr dev, out int level, out IntPtr ctx);
  [DllImport("gdi32.dll")] static extern int D3DKMTOpenAdapterFromLuid([In, Out] byte[] a);
  [DllImport("gdi32.dll")] static extern int D3DKMTCloseAdapter([In, Out] byte[] a);
  [DllImport("gdi32.dll")] static extern int D3DKMTQueryAdapterInfo([In, Out] byte[] a);
  [DllImport("gdi32.dll")] static extern int D3DKMTCreateDevice([In, Out] byte[] a);
  [DllImport("gdi32.dll")] static extern int D3DKMTDestroyDevice([In, Out] byte[] a);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int Fn1(IntPtr self, uint a, out IntPtr p);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnBuf(IntPtr self, byte[] desc);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnQI(IntPtr self, ref Guid riid, out IntPtr p);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnQVMI(IntPtr self, uint node, int group, byte[] info);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate uint FnRelease(IntPtr self);
  static IntPtr Slot(IntPtr obj, int idx) { IntPtr vt = Marshal.ReadIntPtr(obj); return Marshal.ReadIntPtr(vt, idx * IntPtr.Size); }
  static T D<T>(IntPtr obj, int idx) { return (T)(object)Marshal.GetDelegateForFunctionPointer(Slot(obj, idx), typeof(T)); }
  static void P(string s) { Console.WriteLine("VMP " + s); }
  static string H(int hr) { return "0x" + hr.ToString("x8"); }
  static ulong U64(byte[] b, int o) { return BitConverter.ToUInt64(b, o); }
  static string MB(ulong v) { return v + " (" + (v >> 20) + " MB)"; }
  static void Desc(byte[] d, string tag) {
    string name = System.Text.Encoding.Unicode.GetString(d, 0, 256).Split('\0')[0];
    P(tag + " name='" + name + "' vendor=0x" + BitConverter.ToUInt32(d, 256).ToString("x4") + " device=0x" + BitConverter.ToUInt32(d, 260).ToString("x4") + " rev=" + BitConverter.ToUInt32(d, 268));
    P(tag + " DedicatedVideoMemory=" + MB(U64(d, 272)) + " DedicatedSystemMemory=" + MB(U64(d, 280)) + " SharedSystemMemory=" + MB(U64(d, 288)));
    P(tag + " LUID=" + BitConverter.ToUInt32(d, 300).ToString("x") + ":" + BitConverter.ToUInt32(d, 296).ToString("x8") + " Flags=0x" + BitConverter.ToUInt32(d, 304).ToString("x"));
  }
  static void Kmt(byte[] luid, string tag) {
    // D3DKMT_OPENADAPTERFROMLUID { LUID (8); D3DKMT_HANDLE hAdapter (4) }
    byte[] oa = new byte[16]; Array.Copy(luid, 0, oa, 0, 8);
    int hr = D3DKMTOpenAdapterFromLuid(oa); P(tag + " D3DKMTOpenAdapterFromLuid NTSTATUS 0x" + hr.ToString("x8"));
    if (hr != 0) return;
    uint hA = BitConverter.ToUInt32(oa, 8); P(tag + " hAdapter 0x" + hA.ToString("x"));
    // D3DKMT_QUERYADAPTERINFO { hAdapter (4); Type (4); void* pPrivateDriverData (8); UINT size (4) } = 24 bytes (x64)
    int[] types = new int[] { 3, 13, 15 };
    string[] names = new string[] { "GETSEGMENTSIZE", "DRIVERVERSION", "ADAPTERTYPE" };
    for (int i = 0; i < types.Length; i++) {
      IntPtr buf = Marshal.AllocHGlobal(64);
      for (int k = 0; k < 64; k++) Marshal.WriteByte(buf, k, 0);
      byte[] q = new byte[24];
      BitConverter.GetBytes(hA).CopyTo(q, 0); BitConverter.GetBytes(types[i]).CopyTo(q, 4);
      BitConverter.GetBytes(buf.ToInt64()).CopyTo(q, 8); BitConverter.GetBytes(types[i] == 3 ? 24 : 4).CopyTo(q, 16);
      hr = D3DKMTQueryAdapterInfo(q);
      byte[] r = new byte[24]; Marshal.Copy(buf, r, 0, 24); Marshal.FreeHGlobal(buf);
      if (types[i] == 3) P(tag + " KMTQAITYPE_GETSEGMENTSIZE NTSTATUS 0x" + hr.ToString("x8") + " DedicatedVideoMemorySize=" + MB(U64(r, 0)) + " DedicatedSystemMemorySize=" + MB(U64(r, 8)) + " SharedSystemMemorySize=" + MB(U64(r, 16)));
      else P(tag + " KMTQAITYPE_" + names[i] + " NTSTATUS 0x" + hr.ToString("x8") + " value 0x" + BitConverter.ToUInt32(r, 0).ToString("x"));
    }
    // D3DKMT_CREATEDEVICE (x64): { union{hAdapter,pAdapter} (8); Flags (4); hDevice (4); pCommandBuffer (8); size (4)+pad; ... } = 64 bytes
    byte[] cd = new byte[64]; BitConverter.GetBytes(hA).CopyTo(cd, 0);
    hr = D3DKMTCreateDevice(cd); uint hD = BitConverter.ToUInt32(cd, 12);
    P(tag + " D3DKMTCreateDevice NTSTATUS 0x" + hr.ToString("x8") + " hDevice 0x" + hD.ToString("x"));
    if (hr == 0) { byte[] dd = new byte[8]; BitConverter.GetBytes(hD).CopyTo(dd, 0); int hr2 = D3DKMTDestroyDevice(dd); P(tag + " D3DKMTDestroyDevice NTSTATUS 0x" + hr2.ToString("x8")); }
    byte[] ca = new byte[4]; BitConverter.GetBytes(hA).CopyTo(ca, 0); D3DKMTCloseAdapter(ca);
  }
  static void Mem(IntPtr a, string tag) {
    Guid a3 = new Guid("645967A4-1392-4310-A798-8053CE3E93FD");
    IntPtr p3; int hr = D<FnQI>(a, 0)(a, ref a3, out p3);
    P(tag + " QueryInterface(IDXGIAdapter3) " + H(hr)); if (hr != 0) return;
    for (int g = 0; g < 2; g++) {
      byte[] info = new byte[32];
      hr = D<FnQVMI>(p3, 14)(p3, 0, g, info);
      P(tag + " QueryVideoMemoryInfo(node 0, " + (g == 0 ? "LOCAL" : "NON_LOCAL") + ") " + H(hr) + " Budget=" + MB(U64(info, 0)) + " CurrentUsage=" + MB(U64(info, 8)) + " AvailableForReservation=" + MB(U64(info, 16)) + " CurrentReservation=" + MB(U64(info, 24)));
    }
  }
  public static void Run() {
    Guid fac = new Guid("770aae78-f26f-4dba-a829-253c83d1b387"), dev = new Guid("189819f1-1db6-4b57-be54-1821339b85f7");
    IntPtr f; int hr = CreateDXGIFactory1(ref fac, out f); P("CreateDXGIFactory1 " + H(hr)); if (hr != 0) return;
    IntPtr nv = IntPtr.Zero, soft = IntPtr.Zero; byte[] nvLuid = null;
    for (uint i = 0; i < 8; i++) {
      IntPtr a; hr = D<Fn1>(f, 12)(f, i, out a); if (hr != 0) break;
      byte[] d = new byte[312]; D<FnBuf>(a, 10)(a, d);
      string tag = "adapter" + i;
      Desc(d, tag + ".GetDesc1");
      uint ven = BitConverter.ToUInt32(d, 256);
      byte[] d2 = new byte[320];
      try { hr = D<FnBuf>(a, 11)(a, d2); P(tag + ".GetDesc2 " + H(hr) + " GraphicsPreemptionGranularity=" + BitConverter.ToUInt32(d2, 308) + " ComputePreemptionGranularity=" + BitConverter.ToUInt32(d2, 312)); } catch (Exception e) { P(tag + ".GetDesc2 exception " + e.Message); }
      Mem(a, tag);
      byte[] luid = new byte[8]; Array.Copy(d, 296, luid, 0, 8);
      Kmt(luid, tag);
      if (ven == 0x10de && nv == IntPtr.Zero) { nv = a; nvLuid = luid; } else if (ven == 0x1414 && soft == IntPtr.Zero) soft = a;
    }
    if (nv == IntPtr.Zero) { P("no NVIDIA adapter"); return; }
    int[] lv = new int[] { 0xb000, 0xc000, 0xc100, 0xc200 };
    string[] ln = new string[] { "11_0", "12_0", "12_1", "12_2" };
    if (soft != IntPtr.Zero) { IntPtr sd; hr = D3D12CreateDevice(soft, 0xb000, ref dev, out sd); P("control D3D12CreateDevice(Basic Render Driver, 11_0) " + H(hr)); }
    for (int i = 0; i < lv.Length; i++) { IntPtr dv; hr = D3D12CreateDevice(nv, lv[i], ref dev, out dv); P("D3D12CreateDevice(NVIDIA, " + ln[i] + ") " + H(hr)); }
    { IntPtr d11, c11; int l11; hr = D3D11CreateDevice(nv, 0, IntPtr.Zero, 0, IntPtr.Zero, 0, 7, out d11, out l11, out c11); P("D3D11CreateDevice(NVIDIA, hardware) " + H(hr) + " feature level 0x" + l11.ToString("x")); }
    Mem(nv, "after-create");
    P("done");
  }
}
'@
"VMP whoami " + (whoami)
"VMP session " + [System.Diagnostics.Process]::GetCurrentProcess().SessionId
try { [KfVmp]::Run() } catch { "VMP exception " + $_.Exception.Message }
try {
  Get-CimInstance Win32_VideoController | ForEach-Object {
    "VMP Win32_VideoController name='" + $_.Name + "' AdapterRAM=" + $_.AdapterRAM + " VideoProcessor='" + $_.VideoProcessor + "' Status=" + $_.Status + " ConfigManagerErrorCode=" + $_.ConfigManagerErrorCode + " DriverVersion=" + $_.DriverVersion
  }
} catch { "VMP cim exception " + $_.Exception.Message }
try {
  $cls = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
  Get-ChildItem $cls -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match '^\d{4}$' } | ForEach-Object {
    $k = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
    $desc = $k.DriverDesc
    $props = $k.PSObject.Properties | Where-Object { $_.Name -like 'HardwareInformation.*' -or $_.Name -like 'MemorySize' } | ForEach-Object { $_.Name + '=' + ($_.Value -join ',') }
    "VMP class-key " + $_.PSChildName + " DriverDesc='" + $desc + "' " + ($props -join ' ')
  }
} catch { "VMP registry exception " + $_.Exception.Message }
