# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# d3d12_signal_probe.ps1 -- the smallest real D3D12 GPU-work probe, run inside the Windows guest:
# find the NVIDIA adapter (DXGI), create a D3D12 device, a DIRECT queue and a COPY queue, and on each
# queue Signal a fence and wait for it (a semaphore release the GPU must execute). One line per step:
# `PROBE <step> <HRESULT or value>`. Guest-side test code; it reads and writes nothing else.
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KfD3D {
  [DllImport("dxgi.dll")] static extern int CreateDXGIFactory1(ref Guid riid, out IntPtr ppv);
  [DllImport("d3d12.dll")] static extern int D3D12CreateDevice(IntPtr adapter, int level, ref Guid riid, out IntPtr dev);
  [DllImport("d3d11.dll")] static extern int D3D11CreateDevice(IntPtr adapter, int driverType, IntPtr sw, uint flags, IntPtr levels, uint nlevels, uint sdk, out IntPtr dev, out int level, out IntPtr ctx);
  [DllImport("kernel32.dll")] static extern IntPtr CreateEvent(IntPtr a, bool manual, bool init, string name);
  [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr h, uint ms);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int Fn1(IntPtr self, uint a, out IntPtr p);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnDesc1(IntPtr self, byte[] desc);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnQueue(IntPtr self, ref QDesc d, ref Guid riid, out IntPtr q);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnFence(IntPtr self, ulong init, int flags, ref Guid riid, out IntPtr f);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnSignal(IntPtr self, IntPtr fence, ulong v);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnSetEvt(IntPtr self, ulong v, IntPtr evt);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate ulong FnGetCompleted(IntPtr self);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate uint FnRelease(IntPtr self);
  [StructLayout(LayoutKind.Sequential)] struct QDesc { public int Type, Priority, Flags; public uint NodeMask; }
  static IntPtr Slot(IntPtr obj, int idx) { IntPtr vt = Marshal.ReadIntPtr(obj); return Marshal.ReadIntPtr(vt, idx * IntPtr.Size); }
  static T D<T>(IntPtr obj, int idx) { return (T)(object)Marshal.GetDelegateForFunctionPointer(Slot(obj, idx), typeof(T)); }
  static void P(string s) { Console.WriteLine("PROBE " + s); }
  static void Rel(IntPtr o) { if (o != IntPtr.Zero) D<FnRelease>(o, 2)(o); }
  public static void Run() {
    Guid fac = new Guid("770aae78-f26f-4dba-a829-253c83d1b387"), dev = new Guid("189819f1-1db6-4b57-be54-1821339b85f7"),
         que = new Guid("0ec870a6-5d7e-4c22-8cfc-5baae07616ed"), fen = new Guid("0a753dcf-c4d8-4b91-adf6-be5a60d95a76");
    IntPtr f; int hr = CreateDXGIFactory1(ref fac, out f); P("CreateDXGIFactory1 0x" + hr.ToString("x8")); if (hr != 0) return;
    IntPtr nv = IntPtr.Zero, soft = IntPtr.Zero;
    for (uint i = 0; i < 8; i++) {
      IntPtr a; hr = D<Fn1>(f, 12)(f, i, out a); if (hr != 0) break;
      byte[] d = new byte[312]; D<FnDesc1>(a, 10)(a, d);
      string name = System.Text.Encoding.Unicode.GetString(d, 0, 256).Split('\0')[0]; uint ven = BitConverter.ToUInt32(d, 256);
      P("adapter " + i + " vendor 0x" + ven.ToString("x4") + " " + name + " dedicated " + (BitConverter.ToUInt64(d, 272) >> 20) + " MB");
      if (ven == 0x10de && nv == IntPtr.Zero) nv = a; else if (ven == 0x1414 && soft == IntPtr.Zero) soft = a; else Rel(a);
    }
    if (nv == IntPtr.Zero) { P("no NVIDIA adapter"); return; }
    if (soft != IntPtr.Zero) { IntPtr sd; hr = D3D12CreateDevice(soft, 0xb000, ref dev, out sd); P("control D3D12CreateDevice(Basic Render Driver, 11_0) 0x" + hr.ToString("x8")); }
    { IntPtr d11, c11; int lv; hr = D3D11CreateDevice(nv, 0, IntPtr.Zero, 0, IntPtr.Zero, 0, 7, out d11, out lv, out c11); P("D3D11CreateDevice(NVIDIA, hardware) 0x" + hr.ToString("x8") + " feature level 0x" + lv.ToString("x")); }
    IntPtr dv; hr = D3D12CreateDevice(nv, 0xb000, ref dev, out dv); P("D3D12CreateDevice(11_0) 0x" + hr.ToString("x8")); if (hr != 0) return;
    foreach (int type in new int[] { 0, 3 }) {
      string tn = type == 0 ? "DIRECT" : "COPY";
      QDesc q = new QDesc(); q.Type = type; IntPtr cq; hr = D<FnQueue>(dv, 8)(dv, ref q, ref que, out cq); P(tn + " CreateCommandQueue 0x" + hr.ToString("x8")); if (hr != 0) continue;
      IntPtr fc; hr = D<FnFence>(dv, 36)(dv, 0, 0, ref fen, out fc); P(tn + " CreateFence 0x" + hr.ToString("x8")); if (hr != 0) continue;
      IntPtr ev = CreateEvent(IntPtr.Zero, false, false, null);
      for (ulong v = 1; v <= 3; v++) {
        hr = D<FnSignal>(cq, 14)(cq, fc, v); D<FnSetEvt>(fc, 9)(fc, v, ev);
        uint w = WaitForSingleObject(ev, 20000);
        P(tn + " Signal(" + v + ") hr 0x" + hr.ToString("x8") + " wait " + (w == 0 ? "SIGNALED" : "TIMEOUT/" + w) + " completed " + D<FnGetCompleted>(fc, 8)(fc));
        if (w != 0) break;
      }
    }
    P("done");
  }
}
'@
"PROBE whoami " + (whoami)
"PROBE session " + [System.Diagnostics.Process]::GetCurrentProcess().SessionId
(quser 2>&1 | Out-String).Trim() -split "`n" | ForEach-Object { "PROBE quser " + $_.Trim() }
try { [KfD3D]::Run() } catch { "PROBE exception " + $_.Exception.Message }
