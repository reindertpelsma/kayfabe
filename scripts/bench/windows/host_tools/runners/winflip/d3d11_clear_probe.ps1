# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# d3d11_clear_probe.ps1 -- guest-side GPU-work probe (2026-10-08, VFIO DVI reference): on the NVIDIA adapter create a
# D3D11 hardware device, a 256x256 RGBA render target, ClearRenderTargetView to (0.25,0.5,0.75,1), CopyResource to a
# staging texture, Flush, Map(READ) and read pixel (0,0) back -- the clear must have executed on the GPU for the
# expected bytes (40 7f|80 bf ff; the GPU rounds 0.5 to 0x7f) to appear. Then the D3D12 fence probe (d3d12_signal_probe.ps1) covers DIRECT/COPY queue
# signals. One line per step: `PROBE <step> <HRESULT or value>` with the guest UTC time (ETW alignment).
# A clear + copy is not a shader draw; label it as such. Reads and writes nothing else.
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KfClear {
  [DllImport("dxgi.dll")] static extern int CreateDXGIFactory1(ref Guid riid, out IntPtr ppv);
  [DllImport("d3d11.dll")] static extern int D3D11CreateDevice(IntPtr adapter, int driverType, IntPtr sw, uint flags, IntPtr levels, uint nlevels, uint sdk, out IntPtr dev, out int level, out IntPtr ctx);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int Fn1(IntPtr self, uint a, out IntPtr p);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnDesc1(IntPtr self, byte[] desc);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnTex(IntPtr self, uint[] desc, IntPtr init, out IntPtr tex);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnRtv(IntPtr self, IntPtr res, IntPtr desc, out IntPtr rtv);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnClear(IntPtr self, IntPtr rtv, float[] rgba);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnCopy(IntPtr self, IntPtr dst, IntPtr src);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnFlush(IntPtr self);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnMap(IntPtr self, IntPtr res, uint sub, int type, uint flags, out Mapped m);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnUnmap(IntPtr self, IntPtr res, uint sub);
  [StructLayout(LayoutKind.Sequential)] public struct Mapped { public IntPtr pData; public uint RowPitch, DepthPitch; }
  static IntPtr Slot(IntPtr obj, int idx) { IntPtr vt = Marshal.ReadIntPtr(obj); return Marshal.ReadIntPtr(vt, idx * IntPtr.Size); }
  static T D<T>(IntPtr obj, int idx) { return (T)(object)Marshal.GetDelegateForFunctionPointer(Slot(obj, idx), typeof(T)); }
  static void P(string s) { Console.WriteLine("PROBE " + DateTime.UtcNow.ToString("HH:mm:ss.fffffff") + " " + s); }
  public static void Run() {
    Guid fac = new Guid("770aae78-f26f-4dba-a829-253c83d1b387");
    IntPtr f; int hr = CreateDXGIFactory1(ref fac, out f); P("CreateDXGIFactory1 0x" + hr.ToString("x8")); if (hr != 0) return;
    IntPtr nv = IntPtr.Zero;
    for (uint i = 0; i < 8; i++) {
      IntPtr a; hr = D<Fn1>(f, 12)(f, i, out a); if (hr != 0) break;
      byte[] d = new byte[312]; D<FnDesc1>(a, 10)(a, d);
      uint ven = BitConverter.ToUInt32(d, 256);
      if (ven == 0x10de && nv == IntPtr.Zero) nv = a;
    }
    if (nv == IntPtr.Zero) { P("no NVIDIA adapter"); return; }
    IntPtr dev, ctx; int lv;
    hr = D3D11CreateDevice(nv, 0, IntPtr.Zero, 0, IntPtr.Zero, 0, 7, out dev, out lv, out ctx);
    P("D3D11CreateDevice(NVIDIA) 0x" + hr.ToString("x8") + " level 0x" + lv.ToString("x")); if (hr != 0) return;
    // D3D11_TEXTURE2D_DESC: W H Mips Array Format SampleCount SampleQuality Usage BindFlags CPUAccess Misc
    uint[] rt = { 256, 256, 1, 1, 28, 1, 0, 0, 0x20, 0, 0 };
    uint[] st = { 256, 256, 1, 1, 28, 1, 0, 3, 0, 0x20000, 0 };
    IntPtr t1, t2, rtv;
    hr = D<FnTex>(dev, 5)(dev, rt, IntPtr.Zero, out t1); P("CreateTexture2D(RT) 0x" + hr.ToString("x8")); if (hr != 0) return;
    hr = D<FnTex>(dev, 5)(dev, st, IntPtr.Zero, out t2); P("CreateTexture2D(STAGING) 0x" + hr.ToString("x8")); if (hr != 0) return;
    hr = D<FnRtv>(dev, 9)(dev, t1, IntPtr.Zero, out rtv); P("CreateRenderTargetView 0x" + hr.ToString("x8")); if (hr != 0) return;
    for (int round = 1; round <= 3; round++) {
      float[] c = { 0.25f, 0.5f, 0.75f, 1.0f };
      D<FnClear>(ctx, 50)(ctx, rtv, c);
      D<FnCopy>(ctx, 47)(ctx, t2, t1);
      D<FnFlush>(ctx, 111)(ctx);
      P("round " + round + " ClearRenderTargetView+CopyResource+Flush issued");
      Mapped m; hr = D<FnMap>(ctx, 14)(ctx, t2, 0, 1, 0, out m);
      if (hr != 0) { P("Map 0x" + hr.ToString("x8")); return; }
      uint px = (uint)Marshal.ReadInt32(m.pData);
      D<FnUnmap>(ctx, 15)(ctx, t2, 0);
      P("round " + round + " Map(READ) ok pixel0 0x" + px.ToString("x8") + ((px == 0xffbf8040 || px == 0xffbf7f40) ? " EXPECTED" : " UNEXPECTED"));
    }
    P("done");
  }
}
'@
"PROBE whoami " + (whoami) + " session " + [System.Diagnostics.Process]::GetCurrentProcess().SessionId
try { [KfClear]::Run() } catch { "PROBE exception " + $_.Exception.Message }
