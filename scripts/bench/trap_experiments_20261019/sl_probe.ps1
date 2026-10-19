# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_gpu_probe.ps1 [-Out C:\kf\TE\ui.txt] [-Iter 2000]: D3D12 micro-probe, same binary on both sides, run in the interactive session (sn_it.ps1).
# Built on the raw-COM pattern of d3d12_signal_probe.ps1 (vtable slots of ID3D12Device/CommandQueue/Fence/GraphicsCommandList). It separates:
#   A/A2 empty-queue / 64 KiB-copy + Signal round trip, completion seen by POLLING the monitored fence (GetCompletedValue)  = submit + GPU + fence-write visibility
#   B  the same, completion seen by WAITING on an event (SetEventOnCompletion)                         = A + interrupt/DPC/event/wake latency
#   C  VRAM->VRAM CopyResource of 256 MiB, polled  (GPU memory-system speed, GB/s)
#   D  upload-heap (system memory) -> VRAM CopyResource of 128 MiB, polled (PCIe read / IOMMU path, GB/s)
#   E  VRAM -> readback-heap CopyResource of 128 MiB, polled (PCIe write path, GB/s)
#   F  DIRECT -> COPY queue dependency: DIRECT signals F1, COPY waits F1 then signals F2, polled on F2 (GPU/OS cross-queue step latency)
#   G  64 tiny copy lists in ONE ExecuteCommandLists + Signal, polled;  H  64 separate ExecuteCommandLists + one Signal, polled   (per-submission vs per-round-trip cost)
#   I  VRAM->VRAM CopyResource size sweep 64 KiB .. 256 MiB, polled (intercept = fixed latency, slope = GPU bandwidth)
#   J  CPU time inside ExecuteCommandLists / Signal / the wait for the fence
#   T  (only with -WithTimestamps) GPU timestamps: EndQuery(ts0), 64 KiB / 16 MiB copy, EndQuery(ts1) on the DIRECT queue; ts mapped to CPU QPC with GetClockCalibration:
#      submit->GPU-start (ts0 on the CPU clock minus the CPU time before ExecuteCommandLists), GPU execution (ts1-ts0), GPU-end->CPU-visible (poll seen minus ts1)
# Output lines `GP <test> n=.. med=.. p95=.. p99=.. min=..` (us, or ms/GB/s for the copies). Reads/writes nothing outside GPU buffers it creates.
param([string]$Out = 'C:\kf\TE\ui.txt', [int]$Iter = 2000, [switch]$WithTimestamps, [string]$Mode = 'all', [string]$FenceBase = '0', [int]$PreMs = 0, [string]$Raw = '', [int]$Queue = 0, [string]$Work = 'copy', [string]$Seq = '')
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Diagnostics;
using System.Text;
public static class SnGp {
  [DllImport("dxgi.dll")] static extern int CreateDXGIFactory1(ref Guid riid, out IntPtr ppv);
  [DllImport("d3d12.dll")] static extern int D3D12CreateDevice(IntPtr adapter, int level, ref Guid riid, out IntPtr dev);
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
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnAlloc(IntPtr self, int type, ref Guid riid, out IntPtr a);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnList(IntPtr self, uint node, int type, IntPtr alloc, IntPtr pso, ref Guid riid, out IntPtr l);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnRes(IntPtr self, ref HeapProps hp, int hflags, ref ResDesc rd, int state, IntPtr clear, ref Guid riid, out IntPtr r);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnCopyRes(IntPtr self, IntPtr dst, IntPtr src);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnClose(IntPtr self);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnExec(IntPtr self, uint n, IntPtr[] lists);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnQH(IntPtr self, ref QHDesc d, ref Guid riid, out IntPtr h);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnEndQ(IntPtr self, IntPtr heap, int type, uint index);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnResolve(IntPtr self, IntPtr heap, int type, uint start, uint count, IntPtr dst, ulong off);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnMap(IntPtr self, uint sub, IntPtr range, out IntPtr data);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void FnUnmap(IntPtr self, uint sub, IntPtr range);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnTsFreq(IntPtr self, out ulong f);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int FnCal(IntPtr self, out ulong gpu, out ulong cpu);
  [StructLayout(LayoutKind.Sequential)] struct QHDesc { public int Type; public uint Count, Node; }
  [StructLayout(LayoutKind.Sequential)] struct QDesc { public int Type, Priority, Flags; public uint NodeMask; }
  [StructLayout(LayoutKind.Sequential)] struct HeapProps { public int Type, Cpu, Pool; public uint Node, Vis; }
  [StructLayout(LayoutKind.Sequential)] struct ResDesc { public int Dim; public ulong Align, Width; public uint Height; public ushort Depth, Mips; public int Format; public uint SC, SQ; public int Layout, Flags; }
  static IntPtr Slot(IntPtr obj, int idx) { IntPtr vt = Marshal.ReadIntPtr(obj); return Marshal.ReadIntPtr(vt, idx * IntPtr.Size); }
  static T D<T>(IntPtr obj, int idx) { return (T)(object)Marshal.GetDelegateForFunctionPointer(Slot(obj, idx), typeof(T)); }
  static StringBuilder sb = new StringBuilder();
  public static string OutPath = null;
  static void P(string s) { sb.AppendLine("GP " + s); Console.WriteLine("GP " + s); if (OutPath != null) System.IO.File.AppendAllText(OutPath, "GP " + s + "\r\n"); }
  public static string Result() { return sb.ToString(); }
  static double Us(long ticks) { return ticks * 1e6 / Stopwatch.Frequency; }
  static string St(List<double> a, string unit) {
    a.Sort(); int n = a.Count; if (n == 0) return "n=0";
    double mean = 0; foreach (double x in a) mean += x; mean /= n;
    return "n=" + n + " med=" + a[n / 2].ToString("F1") + unit + " mean=" + mean.ToString("F1") + unit + " p95=" + a[(int)(n * 0.95)].ToString("F1") + unit + " p99=" + a[(int)(n * 0.99)].ToString("F1") + unit + " min=" + a[0].ToString("F1") + unit + " max=" + a[n - 1].ToString("F1") + unit;
  }
  static Guid fac = new Guid("770aae78-f26f-4dba-a829-253c83d1b387"), devI = new Guid("189819f1-1db6-4b57-be54-1821339b85f7"),
              queI = new Guid("0ec870a6-5d7e-4c22-8cfc-5baae07616ed"), fenI = new Guid("0a753dcf-c4d8-4b91-adf6-be5a60d95a76"),
              allocI = new Guid("6102dee4-af59-4b09-b999-b44d73f09b24"), listI = new Guid("5b160d0f-ac1b-4185-8ba8-b3ae42a5a455"), resI = new Guid("696442be-a72e-4059-bc79-5b5c98040fad"), qhI = new Guid("0d9658ae-ed45-469e-a61d-970ec583cab4");
  static IntPtr dv;
  static ulong FenceInit = 0;
  static IntPtr Fence() { IntPtr f; int hr = D<FnFence>(dv, 36)(dv, FenceInit, 0, ref fenI, out f); if (hr != 0) P("CreateFence 0x" + hr.ToString("x8")); return f; }
  static IntPtr Queue(int type) { QDesc q = new QDesc(); q.Type = type; IntPtr cq; int hr = D<FnQueue>(dv, 8)(dv, ref q, ref queI, out cq); if (hr != 0) P("CreateCommandQueue 0x" + hr.ToString("x8")); return cq; }
  static IntPtr Buf(int heapType, ulong bytes, int state) {
    HeapProps hp = new HeapProps(); hp.Type = heapType; hp.Node = 1; hp.Vis = 1;
    ResDesc rd = new ResDesc(); rd.Dim = 1; rd.Align = 0; rd.Width = bytes; rd.Height = 1; rd.Depth = 1; rd.Mips = 1; rd.Format = 0; rd.SC = 1; rd.SQ = 0; rd.Layout = 1; rd.Flags = 0;
    IntPtr r; int hr = D<FnRes>(dv, 27)(dv, ref hp, 0, ref rd, state, IntPtr.Zero, ref resI, out r);
    if (hr != 0) { P("CreateCommittedResource heap " + heapType + " " + (bytes >> 20) + "MiB 0x" + hr.ToString("x8")); return IntPtr.Zero; }
    return r;
  }
  static IntPtr CopyList(IntPtr dst, IntPtr src, int ltype = 0, bool empty = false) {
    IntPtr al, l; Guid a = allocI, li = listI;
    D<FnAlloc>(dv, 9)(dv, ltype, ref a, out al); D<FnList>(dv, 12)(dv, 0, ltype, al, IntPtr.Zero, ref li, out l);
    if (!empty) D<FnCopyRes>(l, 17)(l, dst, src);
    D<FnClose>(l, 9)(l); return l;
  }
  static bool PollTo(IntPtr fc, ulong v, double timeoutS) {
    long t0 = Stopwatch.GetTimestamp(); var gc = D<FnGetCompleted>(fc, 8);
    while (gc(fc) < v) { if ((Stopwatch.GetTimestamp() - t0) > timeoutS * Stopwatch.Frequency) return false; }
    return true;
  }
  static void CopyTest(string name, int dstHeap, int srcHeap, int dstState, int srcState, ulong bytes, IntPtr cq, int iters) {
    IntPtr dst = Buf(dstHeap, bytes, dstState), src = Buf(srcHeap, bytes, srcState); if (dst == IntPtr.Zero || src == IntPtr.Zero) return;
    IntPtr l = CopyList(dst, src); IntPtr fc = Fence(); var ex = D<FnExec>(cq, 10); var sg = D<FnSignal>(cq, 14); ulong v = 0;
    List<double> ms = new List<double>();
    for (int i = 0; i < iters + 3; i++) {
      long t0 = Stopwatch.GetTimestamp(); ex(cq, 1, new IntPtr[] { l }); v++; sg(cq, fc, v);
      if (!PollTo(fc, v, 20)) { P(name + " TIMEOUT"); return; }
      long t1 = Stopwatch.GetTimestamp(); if (i >= 3) ms.Add(Us(t1 - t0) / 1000.0);
    }
    List<double> gbs = new List<double>(); foreach (double m in ms) gbs.Add(bytes / 1073741824.0 / (m / 1000.0));
    P(name + " " + (bytes >> 20) + "MiB time " + St(new List<double>(ms), "ms") + " | GiB/s " + St(gbs, ""));
  }
  public static void Run(int iter, bool withTs, string mode, ulong fenceBase, int preMs, string raw, int qtype, string work) {
    FenceInit = fenceBase;
    IntPtr f; int hr = CreateDXGIFactory1(ref fac, out f); if (hr != 0) { P("factory 0x" + hr.ToString("x8")); return; }
    IntPtr nv = IntPtr.Zero;
    for (uint i = 0; i < 8; i++) {
      IntPtr a; hr = D<Fn1>(f, 12)(f, i, out a); if (hr != 0) break;
      byte[] d = new byte[312]; D<FnDesc1>(a, 10)(a, d);
      string name = Encoding.Unicode.GetString(d, 0, 256).Split('\0')[0]; uint ven = BitConverter.ToUInt32(d, 256);
      if (ven == 0x10de && nv == IntPtr.Zero) { nv = a; P("adapter " + name + " dedicated " + (BitConverter.ToUInt64(d, 272) >> 20) + " MiB"); }
    }
    if (nv == IntPtr.Zero) { P("no NVIDIA adapter"); return; }
    hr = D3D12CreateDevice(nv, 0xb000, ref devI, out dv); if (hr != 0) { P("D3D12CreateDevice 0x" + hr.ToString("x8")); return; }
    IntPtr cq = Queue(0), fc = Fence(); IntPtr ev = CreateEvent(IntPtr.Zero, false, false, null);
    var sg = D<FnSignal>(cq, 14); var gc = D<FnGetCompleted>(fc, 8); var se = D<FnSetEvt>(fc, 9);
    ulong v = fenceBase; List<double> pol = new List<double>(), evt = new List<double>(), pol2 = new List<double>(), evt2 = new List<double>();
    IntPtr tb1 = Buf(1, 65536, 0), tb2 = Buf(1, 65536, 0); IntPtr tl = CopyList(tb2, tb1); var ex = D<FnExec>(cq, 10);
    for (int i = 0; i < 200; i++) { v++; sg(cq, fc, v); PollTo(fc, v, 10); }   // warm-up

    if (mode == "T" || mode == "TE") {   // submit-latency probe: per-iteration timestamps (QPC), 64 KiB copy + Signal, polled (T) or event-waited (TE)
      IntPtr[] one = new IntPtr[] { tl };
      if (qtype != 0 || work != "copy") {   // other queue type / empty list: own queue, own fence, own list (fence base kept)
        cq = Queue(qtype); ex = D<FnExec>(cq, 10); sg = D<FnSignal>(cq, 14); fc = Fence(); v = fenceBase; gc = D<FnGetCompleted>(fc, 8); se = D<FnSetEvt>(fc, 9);
        tl = CopyList(tb2, tb1, qtype == 3 ? 3 : (qtype == 2 ? 2 : 0), work == "none"); one = new IntPtr[] { tl };
        for (int i = 0; i < 200; i++) { ex(cq, 1, one); v++; sg(cq, fc, v); PollTo(fc, v, 10); }
      }
      P("T_QUEUE " + qtype + " WORK " + work);
      P("T_FENCE_BASE 0x" + fenceBase.ToString("x") + " value_after_warmup 0x" + v.ToString("x") + " QPCfreq " + Stopwatch.Frequency);
      if (preMs > 0) { P("T_PRE_SLEEP_MS " + preMs); System.Threading.Thread.Sleep(preMs); }
      long[] a = new long[iter], b = new long[iter], c = new long[iter], d = new long[iter];
      P("MODE_" + mode + "_START qpc " + Stopwatch.GetTimestamp() + " utc " + DateTime.UtcNow.ToString("o"));
      for (int i = 0; i < iter; i++) {
        a[i] = Stopwatch.GetTimestamp(); ex(cq, 1, one); b[i] = Stopwatch.GetTimestamp(); v++; sg(cq, fc, v); c[i] = Stopwatch.GetTimestamp();
        if (mode == "T") { if (!PollTo(fc, v, 10)) { P("T TIMEOUT"); return; } }
        else { se(fc, v, ev); if (WaitForSingleObject(ev, 10000) != 0) { P("TE TIMEOUT"); return; } }
        d[i] = Stopwatch.GetTimestamp();
      }
      P("MODE_" + mode + "_END qpc " + Stopwatch.GetTimestamp() + " iterations " + iter);
      List<double> tot = new List<double>(), exl = new List<double>(), sgl = new List<double>(), wt = new List<double>(), s2v = new List<double>(), per = new List<double>();
      for (int i = 5; i < iter; i++) { tot.Add(Us(d[i] - a[i])); exl.Add(Us(b[i] - a[i])); sgl.Add(Us(c[i] - b[i])); wt.Add(Us(d[i] - c[i])); s2v.Add(Us(d[i] - b[i])); if (i > 5) per.Add(Us(a[i] - a[i - 1])); }
      P(mode + " total (Execute call start -> fence visible) us " + St(tot, ""));
      P(mode + " submit-return -> fence visible (the 837-style number) us " + St(s2v, ""));
      P(mode + " ExecuteCommandLists call us " + St(exl, ""));
      P(mode + " Signal call us " + St(sgl, ""));
      P(mode + " wait for fence after Signal returned us " + St(wt, ""));
      P(mode + " iteration period us " + St(per, ""));
      if (!String.IsNullOrEmpty(raw)) {
        StringBuilder r = new StringBuilder("i,qpc_exec_start,qpc_exec_end,qpc_signal_end,qpc_fence_seen\r\n");
        for (int i = 0; i < iter; i++) r.Append(i + "," + a[i] + "," + b[i] + "," + c[i] + "," + d[i] + "\r\n");
        System.IO.File.WriteAllText(raw, r.ToString());
      }
      return;
    }
    if (mode == "A" || mode == "A2") {   // -Mode A / A2: only the empty-Signal / tiny-copy+Signal round trip, repeated (host-side counters are sampled around this run)
      IntPtr[] one = new IntPtr[] { tl };
      P("MODE_" + mode + "_START " + DateTime.UtcNow.ToString("o"));
      for (int i = 0; i < iter; i++) { if (mode == "A2") ex(cq, 1, one); v++; sg(cq, fc, v); PollTo(fc, v, 10); }
      P("MODE_" + mode + "_END " + DateTime.UtcNow.ToString("o") + " iterations " + iter);
      return;
    }
    if (mode == "H") {   // -Mode H: only 64 x ExecuteCommandLists + one Signal, repeated (host-side counters are sampled around this run)
      IntPtr[] one = new IntPtr[] { tl }; List<double> hh = new List<double>();
      P("MODE_H_START " + DateTime.UtcNow.ToString("o"));
      for (int i = 0; i < iter; i++) { long t0 = Stopwatch.GetTimestamp(); for (int k = 0; k < 64; k++) ex(cq, 1, one); v++; sg(cq, fc, v); PollTo(fc, v, 10); hh.Add(Us(Stopwatch.GetTimestamp() - t0)); }
      P("MODE_H_END " + DateTime.UtcNow.ToString("o") + " iterations " + iter + " executes " + (iter * 64) + " signals " + iter);
      P("H  64 x ExecuteCommandLists(1 list) + one Signal, polled  us " + St(hh, ""));
      return;
    }
    for (int round = 0; round < 3; round++) {   // interleave so a slow phase hits every variant
      pol.Clear(); evt.Clear(); pol2.Clear(); evt2.Clear();
      for (int i = 0; i < iter; i++) { long t0 = Stopwatch.GetTimestamp(); v++; sg(cq, fc, v); if (!PollTo(fc, v, 10)) { P("A TIMEOUT"); return; } pol.Add(Us(Stopwatch.GetTimestamp() - t0)); }
      for (int i = 0; i < iter; i++) { long t0 = Stopwatch.GetTimestamp(); v++; sg(cq, fc, v); se(fc, v, ev); if (WaitForSingleObject(ev, 10000) != 0) { P("B TIMEOUT"); return; } evt.Add(Us(Stopwatch.GetTimestamp() - t0)); }
      for (int i = 0; i < iter; i++) { long t0 = Stopwatch.GetTimestamp(); ex(cq, 1, new IntPtr[] { tl }); v++; sg(cq, fc, v); if (!PollTo(fc, v, 10)) { P("A2 TIMEOUT"); return; } pol2.Add(Us(Stopwatch.GetTimestamp() - t0)); }
      for (int i = 0; i < iter; i++) { long t0 = Stopwatch.GetTimestamp(); ex(cq, 1, new IntPtr[] { tl }); v++; sg(cq, fc, v); se(fc, v, ev); if (WaitForSingleObject(ev, 10000) != 0) { P("B2 TIMEOUT"); return; } evt2.Add(Us(Stopwatch.GetTimestamp() - t0)); }
      P("A  empty-queue Signal round trip, polled      round" + round + " us " + St(new List<double>(pol), ""));
      P("B  empty-queue Signal round trip, event-wait  round" + round + " us " + St(new List<double>(evt), ""));
      P("A2 64KiB-copy + Signal round trip, polled     round" + round + " us " + St(new List<double>(pol2), ""));
      P("B2 64KiB-copy + Signal round trip, event-wait round" + round + " us " + St(new List<double>(evt2), ""));
    }
    CopyTest("C vram->vram copy", 1, 1, 0, 0, 256UL << 20, cq, 30);
    CopyTest("D upload(sysmem)->vram copy", 1, 2, 0, 0xAC3, 128UL << 20, cq, 30);
    CopyTest("E vram->readback copy", 3, 1, 0x400 /*COPY_DEST*/, 0, 128UL << 20, cq, 30);
    // F: DIRECT signals F1, COPY queue waits F1 and signals F2; poll F2
    IntPtr cc = Queue(3), f1 = Fence(), f2 = Fence(); var wq = D<FnSignal>(cc, 15); var sc = D<FnSignal>(cc, 14); var sd = D<FnSignal>(cq, 14); var g2 = D<FnGetCompleted>(f2, 8);
    List<double> chain = new List<double>(); ulong w = 0;
    for (int i = 0; i < iter + 100; i++) {
      long t0 = Stopwatch.GetTimestamp(); w++; sd(cq, f1, w); wq(cc, f1, w); sc(cc, f2, w);
      while (g2(f2) < w) { if ((Stopwatch.GetTimestamp() - t0) > 10 * Stopwatch.Frequency) { P("F TIMEOUT"); return; } }
      if (i >= 100) chain.Add(Us(Stopwatch.GetTimestamp() - t0));
    }
    P("F DIRECT-signal -> COPY-wait -> COPY-signal, polled us " + St(chain, ""));

    // G/H: per-submission vs per-round-trip cost
    {
      IntPtr[] ls = new IntPtr[64]; for (int i = 0; i < 64; i++) ls[i] = tl;
      List<double> g = new List<double>(), h = new List<double>(), g1 = new List<double>();
      for (int i = 0; i < 100; i++) {
        long t0 = Stopwatch.GetTimestamp(); ex(cq, 64, ls); v++; sg(cq, fc, v); PollTo(fc, v, 10); g.Add(Us(Stopwatch.GetTimestamp() - t0));
        t0 = Stopwatch.GetTimestamp(); for (int k = 0; k < 64; k++) ex(cq, 1, new IntPtr[] { tl }); v++; sg(cq, fc, v); PollTo(fc, v, 10); h.Add(Us(Stopwatch.GetTimestamp() - t0));
        t0 = Stopwatch.GetTimestamp(); ex(cq, 1, new IntPtr[] { tl }); v++; sg(cq, fc, v); PollTo(fc, v, 10); g1.Add(Us(Stopwatch.GetTimestamp() - t0));
      }
      P("G  64 lists in ONE ExecuteCommandLists + Signal, polled   us " + St(g, ""));
      P("H  64 x ExecuteCommandLists(1 list) + one Signal, polled  us " + St(h, ""));
      P("G0 1 list + Signal, polled (same loop)                    us " + St(g1, ""));
    }
    // I: size sweep
    foreach (ulong mb in new ulong[] { 0, 1, 4, 16, 64, 256 }) {
      ulong bytes = mb == 0 ? 65536UL : (mb << 20);
      IntPtr a = Buf(1, bytes, 0), b = Buf(1, bytes, 0); if (a == IntPtr.Zero || b == IntPtr.Zero) continue; IntPtr l2 = CopyList(b, a); List<double> t = new List<double>();
      for (int i = 0; i < 43; i++) { long t0 = Stopwatch.GetTimestamp(); ex(cq, 1, new IntPtr[] { l2 }); v++; sg(cq, fc, v); PollTo(fc, v, 10); if (i >= 3) t.Add(Us(Stopwatch.GetTimestamp() - t0)); }
      P("I  vram->vram " + (mb == 0 ? "64KiB" : mb + "MiB") + " round trip us " + St(t, ""));
    }
    // J: CPU time inside the calls: ExecuteCommandLists(1 tiny list), Signal, then GPU wait (poll)
    {
      List<double> te = new List<double>(), ts = new List<double>(), tp = new List<double>();
      for (int i = 0; i < 1500; i++) {
        long t0 = Stopwatch.GetTimestamp(); ex(cq, 1, new IntPtr[] { tl }); long t1 = Stopwatch.GetTimestamp(); v++; sg(cq, fc, v); long t2 = Stopwatch.GetTimestamp(); PollTo(fc, v, 10); long t3 = Stopwatch.GetTimestamp();
        if (i >= 50) { te.Add(Us(t1 - t0)); ts.Add(Us(t2 - t1)); tp.Add(Us(t3 - t2)); }
      }
      P("J  CPU time inside ExecuteCommandLists(1 tiny list) us " + St(te, ""));
      P("J  CPU time inside Signal us " + St(ts, ""));
      P("J  then wait for the fence (poll) us " + St(tp, ""));
    }
    // T: GPU timestamps (only with -WithTimestamps: on the kayfabe guest ResolveQueryData into a readback buffer made the D3D12 device report DEVICE_REMOVED)
    if (withTs) {
      IntPtr qh; QHDesc qd = new QHDesc(); qd.Type = 1; qd.Count = 2; hr = D<FnQH>(dv, 39)(dv, ref qd, ref qhI, out qh); P("CreateQueryHeap 0x" + hr.ToString("x8"));
      IntPtr rb = Buf(3, 4096, 0x400); P("T readback buffer " + rb); ulong gf; D<FnTsFreq>(cq, 16)(cq, out gf); P("GPU timestamp frequency " + gf + " Hz, QPC " + Stopwatch.Frequency + " Hz");
      foreach (ulong bytes in new ulong[] { 65536UL, 16UL << 20 }) {
        IntPtr a = Buf(1, bytes, 0), b = Buf(1, bytes, 0), al, l; Guid ai = allocI, li = listI;
        D<FnAlloc>(dv, 9)(dv, 0, ref ai, out al); D<FnList>(dv, 12)(dv, 0, 0, al, IntPtr.Zero, ref li, out l);
        D<FnEndQ>(l, 53)(l, qh, 1, 0); D<FnCopyRes>(l, 17)(l, b, a); D<FnEndQ>(l, 53)(l, qh, 1, 1); D<FnResolve>(l, 54)(l, qh, 1, 0, 2, rb, 0); D<FnClose>(l, 9)(l); P("T list recorded " + bytes);
        List<double> s2g = new List<double>(), gx = new List<double>(), g2v = new List<double>(), rtt = new List<double>();
        for (int i = 0; i < 400; i++) {
          long t0 = Stopwatch.GetTimestamp(); ex(cq, 1, new IntPtr[] { l }); v++; sg(cq, fc, v); PollTo(fc, v, 10); long t1 = Stopwatch.GetTimestamp();
          if (i == 0) P("T first iteration executed"); ulong gpuC, cpuC; int hrc = D<FnCal>(cq, 17)(cq, out gpuC, out cpuC); if (i == 0) P("T calibration hr 0x" + hrc.ToString("x8") + " gpu " + gpuC + " cpu " + cpuC);
          IntPtr data; int hrm = D<FnMap>(rb, 8)(rb, 0, IntPtr.Zero, out data); if (i == 0) P("T Map hr 0x" + hrm.ToString("x8") + " ptr " + data); if (hrm != 0 || data == IntPtr.Zero) { P("T Map failed"); return; } long g0 = Marshal.ReadInt64(data), g1v = Marshal.ReadInt64(data, 8); D<FnUnmap>(rb, 9)(rb, 0, IntPtr.Zero);
          if (i < 20) continue;
          double qpcPerGpu = (double)Stopwatch.Frequency / gf;
          double cpu0 = (double)cpuC + (g0 - (long)gpuC) * qpcPerGpu, cpu1 = (double)cpuC + (g1v - (long)gpuC) * qpcPerGpu;
          s2g.Add(Us((long)(cpu0 - t0))); gx.Add((g1v - g0) * 1e6 / gf); g2v.Add(Us((long)(t1 - cpu1))); rtt.Add(Us(t1 - t0));
        }
        string nm = bytes == 65536UL ? "64KiB" : "16MiB";
        P("T  " + nm + " copy: submit->GPU-start (ts0 on CPU clock - CPU before Execute) us " + St(s2g, ""));
        P("T  " + nm + " copy: GPU execution ts1-ts0 us " + St(gx, ""));
        P("T  " + nm + " copy: GPU-end(ts1) -> CPU poll saw fence us " + St(g2v, ""));
        P("T  " + nm + " copy: whole round trip us " + St(rtt, ""));
      }
    }
    P("done");
  }
}
'@
Remove-Item $Out -ErrorAction SilentlyContinue
[SnGp]::OutPath = $Out
if ($Seq -ne '') {   # -Seq "T:0:copy:3000,T:3:copy:3000,..." : several runs in ONE process (one guest launch), 4 s apart; raw file per step
  $k = 0
  foreach ($st in $Seq.Split(',')) { $a = $st.Split(':'); $k++
    try { [SnGp]::Run([int]$a[3], $false, $a[0], [Convert]::ToUInt64($FenceBase, 16), $(if ($k -eq 1) { $PreMs } else { 0 }), $(if ($Raw) { $Raw + ".$k" } else { '' }), [int]$a[1], $a[2]) } catch { "GP exception " + $_.Exception.ToString() | Add-Content $Out }
    "GP SEQ_STEP_DONE $k $st" | Add-Content $Out; Start-Sleep -Seconds 4 }
  "GP iteration period SEQ_END" | Add-Content $Out
} else {
try { [SnGp]::Run($Iter, [bool]$WithTimestamps, $Mode, [Convert]::ToUInt64($FenceBase, 16), $PreMs, $Raw, $Queue, $Work) } catch { "GP exception " + $_.Exception.ToString() | Add-Content $Out }
'GP end' | Add-Content $Out
}
