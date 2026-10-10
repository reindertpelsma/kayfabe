// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// kf_dxprobe.cpp -- Direct3D 11 / Direct3D 12 GPU-work probe for the Windows app matrix.
//   kf_dxprobe.exe d3d11|d3d12 [--any-vendor] [--iters N]
// Enumerates the DXGI adapters, picks the NVIDIA one (VendorId 0x10DE; the Microsoft Basic Render
// Driver / WARP is NEVER used, so a software fallback cannot pass), creates a hardware device on it,
// runs a compute shader (and, for D3D11, a draw) whose result is read back and compared, and waits
// on real GPU fences. Output is line oriented: `KFDX <key> <value...>`, the verdict is the last
// line `KFDX RESULT OK|FAIL <why>`; exit code 0 iff OK. --any-vendor is for running the probe
// under Wine on a GPU-less machine (logic test only); the matrix never passes it.
// Shaders are compiled at run time with the system's d3dcompiler_47.dll (SM 5.0, DXBC), so the guest
// needs no SDK. Build: build_tools.sh (mingw-w64, static).
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <d3d11.h>
#include <d3d12.h>
#include <dxgi1_4.h>
#include <d3dcompiler.h>
#include <stdio.h>
#include <string.h>
#include <string>
#include <vector>

static int g_fail = 0;
static std::string g_why;
static void say(const char *k, const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); printf("KFDX %s ", k); vprintf(fmt, ap); printf("\n"); fflush(stdout); va_end(ap);
}
static bool check(bool ok, const char *what, HRESULT hr = 0) {
  if (ok) { say("ok", "%s", what); return true; }
  say("FAILED", "%s hr=0x%08lx", what, (unsigned long)hr);
  if (!g_fail) { g_fail = 1; g_why = what; }
  return false;
}
template <class T> static void rel(T *&p) { if (p) { p->Release(); p = nullptr; } }

typedef HRESULT(WINAPI *PFN_D3DCompile_t)(LPCVOID, SIZE_T, LPCSTR, const D3D_SHADER_MACRO *, ID3DInclude *, LPCSTR, LPCSTR, UINT, UINT, ID3DBlob **, ID3DBlob **);
static ID3DBlob *compile(const char *src, const char *entry, const char *target) {
  static PFN_D3DCompile_t fn = nullptr;
  if (!fn) {
    HMODULE m = LoadLibraryA("d3dcompiler_47.dll");
    if (m) fn = (PFN_D3DCompile_t)GetProcAddress(m, "D3DCompile");
  }
  if (!fn) { say("compile", "d3dcompiler_47.dll missing"); return nullptr; }
  ID3DBlob *code = nullptr, *err = nullptr;
  HRESULT hr = fn(src, strlen(src), "kf", nullptr, nullptr, entry, target, 0, 0, &code, &err);
  if (FAILED(hr)) { say("compile", "%s %s failed 0x%08lx: %s", entry, target, (unsigned long)hr, err ? (const char *)err->GetBufferPointer() : "?"); }
  rel(err);
  return code;
}

static std::string narrow(const wchar_t *w) {
  char b[256]; WideCharToMultiByte(CP_UTF8, 0, w, -1, b, sizeof b, nullptr, nullptr); return b;
}

// the NVIDIA adapter (or, with --any-vendor, the first non-Basic-Render one)
static IDXGIAdapter1 *pick_adapter(bool any, std::string &name) {
  IDXGIFactory1 *f = nullptr;
  if (!check(SUCCEEDED(CreateDXGIFactory1(__uuidof(IDXGIFactory1), (void **)&f)), "CreateDXGIFactory1")) return nullptr;
  IDXGIAdapter1 *pick = nullptr;
  for (UINT i = 0;; i++) {
    IDXGIAdapter1 *a = nullptr;
    if (f->EnumAdapters1(i, &a) == DXGI_ERROR_NOT_FOUND) break;
    DXGI_ADAPTER_DESC1 d; a->GetDesc1(&d);
    std::string n = narrow(d.Description);
    bool soft = (d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) || d.VendorId == 0x1414;
    say("adapter", "index=%u vendor=0x%04x device=0x%04x luid=0x%08lx_0x%08lx dedicated_mb=%llu software=%d name=\"%s\"", i, d.VendorId,
        d.DeviceId, (unsigned long)d.AdapterLuid.HighPart, (unsigned long)d.AdapterLuid.LowPart, (unsigned long long)(d.DedicatedVideoMemory >> 20), soft ? 1 : 0, n.c_str());
    if (!pick && !soft && (d.VendorId == 0x10DE || any)) { pick = a; name = n; } else a->Release();
  }
  f->Release();
  if (!pick) check(false, "an NVIDIA hardware adapter exists (WARP/Basic Render excluded)");
  return pick;
}

// ------------------------------------------------------------------------------------- D3D11
static const char *CS11 =
    "RWStructuredBuffer<uint> o : register(u0);\n"
    "[numthreads(64,1,1)] void main(uint3 t : SV_DispatchThreadID) { o[t.x] = t.x * 3u + 7u; }\n";
// fullscreen triangle, CLOCKWISE in NDC (D3D11 default rasterizer culls back faces, front = clockwise: the previous
// vertex order was counter-clockwise, was culled and left the clear colour in the readback on every hardware adapter)
static const char *VS11 = "float4 main(uint id : SV_VertexID) : SV_Position { float2 p = float2(id & 2, (id << 1) & 2); return float4(p * 2.0 - 1.0, 0, 1); }\n";
static const char *PS11 = "float4 main(float4 p : SV_Position) : SV_Target { return float4(0.25, 0.5, 0.75, 1.0); }\n";

static int run_d3d11(IDXGIAdapter1 *ad, int iters) {
  ID3D11Device *dev = nullptr; ID3D11DeviceContext *ctx = nullptr; D3D_FEATURE_LEVEL fl = (D3D_FEATURE_LEVEL)0;
  D3D_FEATURE_LEVEL want[] = {D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0};
  HRESULT hr = D3D11CreateDevice(ad, D3D_DRIVER_TYPE_UNKNOWN, nullptr, 0, want, 2, D3D11_SDK_VERSION, &dev, &fl, &ctx);
  if (!check(SUCCEEDED(hr), "D3D11CreateDevice(hardware, chosen adapter)", hr)) return 1;
  say("feature_level", "0x%x", (unsigned)fl);
  // compute: 4096 threads write t*3+7 into a UAV buffer; copy to staging; compare
  const UINT N = 4096;
  ID3DBlob *cs = compile(CS11, "main", "cs_5_0");
  if (check(cs != nullptr, "compile cs_5_0")) {
    ID3D11ComputeShader *csh = nullptr;
    check(SUCCEEDED(dev->CreateComputeShader(cs->GetBufferPointer(), cs->GetBufferSize(), nullptr, &csh)), "CreateComputeShader");
    D3D11_BUFFER_DESC bd = {}; bd.ByteWidth = N * 4; bd.Usage = D3D11_USAGE_DEFAULT; bd.BindFlags = D3D11_BIND_UNORDERED_ACCESS;
    bd.MiscFlags = D3D11_RESOURCE_MISC_BUFFER_STRUCTURED; bd.StructureByteStride = 4;
    ID3D11Buffer *buf = nullptr, *stg = nullptr;
    dev->CreateBuffer(&bd, nullptr, &buf);
    bd.Usage = D3D11_USAGE_STAGING; bd.BindFlags = 0; bd.MiscFlags = 0; bd.StructureByteStride = 0; bd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    dev->CreateBuffer(&bd, nullptr, &stg);
    D3D11_UNORDERED_ACCESS_VIEW_DESC ud = {}; ud.Format = DXGI_FORMAT_UNKNOWN; ud.ViewDimension = D3D11_UAV_DIMENSION_BUFFER; ud.Buffer.NumElements = N;
    ID3D11UnorderedAccessView *uav = nullptr;
    if (csh && buf && stg && check(SUCCEEDED(dev->CreateUnorderedAccessView(buf, &ud, &uav)), "CreateUnorderedAccessView")) {
      DWORD t0 = GetTickCount();
      int bad = 0;
      for (int it = 0; it < iters; it++) {
        ctx->CSSetShader(csh, nullptr, 0);
        ctx->CSSetUnorderedAccessViews(0, 1, &uav, nullptr);
        ctx->Dispatch(N / 64, 1, 1);
        ctx->CopyResource(stg, buf);
        D3D11_MAPPED_SUBRESOURCE m;
        if (FAILED(ctx->Map(stg, 0, D3D11_MAP_READ, 0, &m))) { bad = -1; break; }
        const unsigned *p = (const unsigned *)m.pData;
        for (UINT i = 0; i < N; i++) if (p[i] != i * 3u + 7u) bad++;
        ctx->Unmap(stg, 0);
        if (bad) break;
      }
      say("compute", "iters=%d bad=%d ms=%lu", iters, bad, (unsigned long)(GetTickCount() - t0));
      check(bad == 0, "compute shader result matches (UAV readback)");
    }
    rel(uav); rel(buf); rel(stg); rel(csh); rel(cs);
  }
  // draw: a fullscreen triangle into a 256x256 render target, read the centre pixel back
  ID3DBlob *vs = compile(VS11, "main", "vs_5_0"), *ps = compile(PS11, "main", "ps_5_0");
  if (check(vs && ps, "compile vs_5_0/ps_5_0")) {
    ID3D11VertexShader *vsh = nullptr; ID3D11PixelShader *psh = nullptr;
    dev->CreateVertexShader(vs->GetBufferPointer(), vs->GetBufferSize(), nullptr, &vsh);
    dev->CreatePixelShader(ps->GetBufferPointer(), ps->GetBufferSize(), nullptr, &psh);
    D3D11_TEXTURE2D_DESC td = {}; td.Width = td.Height = 256; td.MipLevels = td.ArraySize = 1; td.Format = DXGI_FORMAT_R8G8B8A8_UNORM; td.SampleDesc.Count = 1;
    td.Usage = D3D11_USAGE_DEFAULT; td.BindFlags = D3D11_BIND_RENDER_TARGET;
    ID3D11Texture2D *rt = nullptr, *st = nullptr; ID3D11RenderTargetView *rtv = nullptr;
    dev->CreateTexture2D(&td, nullptr, &rt);
    td.Usage = D3D11_USAGE_STAGING; td.BindFlags = 0; td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    dev->CreateTexture2D(&td, nullptr, &st);
    if (vsh && psh && rt && st && SUCCEEDED(dev->CreateRenderTargetView(rt, nullptr, &rtv))) {
      float clear[4] = {0, 0, 0, 1};
      ctx->ClearRenderTargetView(rtv, clear);
      ctx->OMSetRenderTargets(1, &rtv, nullptr);
      D3D11_VIEWPORT vp = {0, 0, 256, 256, 0, 1}; ctx->RSSetViewports(1, &vp);
      ctx->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
      ctx->VSSetShader(vsh, nullptr, 0); ctx->PSSetShader(psh, nullptr, 0);
      ctx->Draw(3, 0);
      ctx->CopyResource(st, rt);
      D3D11_MAPPED_SUBRESOURCE m;
      if (check(SUCCEEDED(ctx->Map(st, 0, D3D11_MAP_READ, 0, &m)), "Map staging texture")) {
        const unsigned char *px = (const unsigned char *)m.pData + 128 * m.RowPitch + 128 * 4;
        say("pixel", "%u,%u,%u,%u", px[0], px[1], px[2], px[3]);
        check(abs(px[0] - 64) <= 1 && abs(px[1] - 128) <= 1 && abs(px[2] - 191) <= 1 && px[3] == 255, "draw result matches (centre pixel 64,128,191,255)");
        ctx->Unmap(st, 0);
      }
    }
    rel(rtv); rel(rt); rel(st); rel(vsh); rel(psh);
  }
  rel(vs); rel(ps); rel(ctx); rel(dev);
  return g_fail;
}

// ------------------------------------------------------------------------------------- D3D12
static const char *CS12 =
    "RWByteAddressBuffer o : register(u0);\n"
    "[numthreads(64,1,1)] void main(uint3 t : SV_DispatchThreadID) { o.Store(t.x * 4, t.x * 5u + 11u); }\n";

static bool wait_fence(ID3D12Fence *f, UINT64 v, DWORD ms) {
  HANDLE e = CreateEventA(nullptr, FALSE, FALSE, nullptr);
  f->SetEventOnCompletion(v, e);
  DWORD w = WaitForSingleObject(e, ms);
  CloseHandle(e);
  return w == WAIT_OBJECT_0;
}

static int run_d3d12(IDXGIAdapter1 *ad, int iters) {
  ID3D12Device *dev = nullptr;
  D3D_FEATURE_LEVEL levels[] = {D3D_FEATURE_LEVEL_12_2, D3D_FEATURE_LEVEL_12_1, D3D_FEATURE_LEVEL_12_0, D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0};
  D3D_FEATURE_LEVEL got = (D3D_FEATURE_LEVEL)0;
  for (auto l : levels) {
    HRESULT hr = D3D12CreateDevice(ad, l, __uuidof(ID3D12Device), (void **)&dev);
    if (SUCCEEDED(hr)) { got = l; break; }
  }
  if (!check(dev != nullptr, "D3D12CreateDevice(chosen adapter)")) return 1;
  say("feature_level", "0x%x", (unsigned)got);
  IDXGIAdapter3 *a3 = nullptr;
  if (SUCCEEDED(ad->QueryInterface(__uuidof(IDXGIAdapter3), (void **)&a3))) {
    DXGI_QUERY_VIDEO_MEMORY_INFO mi = {};
    if (SUCCEEDED(a3->QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mi))) say("vidmem", "budget_mb=%llu usage_mb=%llu", (unsigned long long)(mi.Budget >> 20), (unsigned long long)(mi.CurrentUsage >> 20));
    a3->Release();
  }
  // DIRECT and COPY queues: signal a fence on each and wait for it (the GPU must execute the signal)
  const struct { D3D12_COMMAND_LIST_TYPE t; const char *n; } qs[] = {{D3D12_COMMAND_LIST_TYPE_DIRECT, "DIRECT"}, {D3D12_COMMAND_LIST_TYPE_COPY, "COPY"}};
  for (auto &q : qs) {
    D3D12_COMMAND_QUEUE_DESC qd = {}; qd.Type = q.t;
    ID3D12CommandQueue *cq = nullptr; ID3D12Fence *fc = nullptr;
    if (!check(SUCCEEDED(dev->CreateCommandQueue(&qd, __uuidof(ID3D12CommandQueue), (void **)&cq)), q.n)) continue;
    dev->CreateFence(0, D3D12_FENCE_FLAG_NONE, __uuidof(ID3D12Fence), (void **)&fc);
    bool all = true;
    for (UINT64 v = 1; v <= 3; v++) { cq->Signal(fc, v); all = all && wait_fence(fc, v, 20000); }
    char b[64]; snprintf(b, sizeof b, "%s queue fence Signal/Wait x3", q.n); check(all, b);
    rel(fc); rel(cq);
  }
  // compute: root UAV (RWByteAddressBuffer), 4096 threads write t*5+11, copy to a readback buffer, compare
  const UINT N = 4096;
  ID3DBlob *cs = compile(CS12, "main", "cs_5_0");
  if (check(cs != nullptr, "compile cs_5_0")) {
    D3D12_ROOT_PARAMETER rp = {}; rp.ParameterType = D3D12_ROOT_PARAMETER_TYPE_UAV; rp.Descriptor.ShaderRegister = 0; rp.ShaderVisibility = D3D12_SHADER_VISIBILITY_ALL;
    D3D12_ROOT_SIGNATURE_DESC rsd = {}; rsd.NumParameters = 1; rsd.pParameters = &rp;
    ID3DBlob *sig = nullptr, *err = nullptr;
    HRESULT hr = D3D12SerializeRootSignature(&rsd, D3D_ROOT_SIGNATURE_VERSION_1, &sig, &err);
    ID3D12RootSignature *rs = nullptr; ID3D12PipelineState *pso = nullptr;
    if (check(SUCCEEDED(hr), "serialize root signature", hr)) {
      check(SUCCEEDED(dev->CreateRootSignature(0, sig->GetBufferPointer(), sig->GetBufferSize(), __uuidof(ID3D12RootSignature), (void **)&rs)), "CreateRootSignature");
      D3D12_COMPUTE_PIPELINE_STATE_DESC pd = {}; pd.pRootSignature = rs; pd.CS = {cs->GetBufferPointer(), cs->GetBufferSize()};
      hr = dev->CreateComputePipelineState(&pd, __uuidof(ID3D12PipelineState), (void **)&pso);
      check(SUCCEEDED(hr), "CreateComputePipelineState", hr);
    }
    D3D12_HEAP_PROPERTIES hdef = {D3D12_HEAP_TYPE_DEFAULT}, hrb = {D3D12_HEAP_TYPE_READBACK};
    D3D12_RESOURCE_DESC rd = {}; rd.Dimension = D3D12_RESOURCE_DIMENSION_BUFFER; rd.Width = N * 4; rd.Height = rd.DepthOrArraySize = rd.MipLevels = 1;
    rd.SampleDesc.Count = 1; rd.Layout = D3D12_TEXTURE_LAYOUT_ROW_MAJOR; rd.Flags = D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
    ID3D12Resource *buf = nullptr, *rb = nullptr;
    dev->CreateCommittedResource(&hdef, D3D12_HEAP_FLAG_NONE, &rd, D3D12_RESOURCE_STATE_UNORDERED_ACCESS, nullptr, __uuidof(ID3D12Resource), (void **)&buf);
    rd.Flags = D3D12_RESOURCE_FLAG_NONE;
    dev->CreateCommittedResource(&hrb, D3D12_HEAP_FLAG_NONE, &rd, D3D12_RESOURCE_STATE_COPY_DEST, nullptr, __uuidof(ID3D12Resource), (void **)&rb);
    D3D12_COMMAND_QUEUE_DESC qd = {}; qd.Type = D3D12_COMMAND_LIST_TYPE_DIRECT;
    ID3D12CommandQueue *cq = nullptr; ID3D12CommandAllocator *al = nullptr; ID3D12GraphicsCommandList *cl = nullptr; ID3D12Fence *fc = nullptr;
    dev->CreateCommandQueue(&qd, __uuidof(ID3D12CommandQueue), (void **)&cq);
    dev->CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT, __uuidof(ID3D12CommandAllocator), (void **)&al);
    dev->CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, al, pso, __uuidof(ID3D12GraphicsCommandList), (void **)&cl);
    dev->CreateFence(0, D3D12_FENCE_FLAG_NONE, __uuidof(ID3D12Fence), (void **)&fc);
    if (pso && buf && rb && cq && cl && fc) {
      DWORD t0 = GetTickCount(); int bad = 0; UINT64 fv = 0; bool waited = true;
      for (int it = 0; it < iters && waited && !bad; it++) {
        if (it) { al->Reset(); cl->Reset(al, pso); }
        cl->SetComputeRootSignature(rs);
        cl->SetPipelineState(pso);
        cl->SetComputeRootUnorderedAccessView(0, buf->GetGPUVirtualAddress());
        cl->Dispatch(N / 64, 1, 1);
        D3D12_RESOURCE_BARRIER b = {}; b.Type = D3D12_RESOURCE_BARRIER_TYPE_TRANSITION; b.Transition.pResource = buf;
        b.Transition.StateBefore = D3D12_RESOURCE_STATE_UNORDERED_ACCESS; b.Transition.StateAfter = D3D12_RESOURCE_STATE_COPY_SOURCE;
        b.Transition.Subresource = D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES;
        cl->ResourceBarrier(1, &b);
        cl->CopyResource(rb, buf);
        std::swap(b.Transition.StateBefore, b.Transition.StateAfter);
        cl->ResourceBarrier(1, &b);
        cl->Close();
        ID3D12CommandList *lists[] = {cl};
        cq->ExecuteCommandLists(1, lists);
        cq->Signal(fc, ++fv);
        waited = wait_fence(fc, fv, 30000);
        if (!waited) break;
        void *p = nullptr; D3D12_RANGE rr = {0, N * 4};
        if (FAILED(rb->Map(0, &rr, &p))) { bad = -1; break; }
        const unsigned *u = (const unsigned *)p;
        for (UINT i = 0; i < N; i++) if (u[i] != i * 5u + 11u) bad++;
        D3D12_RANGE wr = {0, 0}; rb->Unmap(0, &wr);
      }
      say("compute", "iters=%d bad=%d fence_wait_ok=%d ms=%lu", iters, bad, waited ? 1 : 0, (unsigned long)(GetTickCount() - t0));
      check(waited, "compute queue fence completed");
      check(bad == 0, "compute shader result matches (UAV copy-back)");
    } else check(false, "D3D12 compute objects created");
    rel(fc); rel(cl); rel(al); rel(cq); rel(rb); rel(buf); rel(pso); rel(rs); rel(sig); rel(err); rel(cs);
  }
  rel(dev);
  return g_fail;
}

int main(int argc, char **argv) {
  const char *api = argc > 1 ? argv[1] : "";
  bool any = false; int iters = 50;
  for (int i = 2; i < argc; i++) {
    if (!strcmp(argv[i], "--any-vendor")) any = true;
    else if (!strcmp(argv[i], "--iters") && i + 1 < argc) iters = atoi(argv[++i]);
  }
  if (strcmp(api, "d3d11") && strcmp(api, "d3d12")) { fprintf(stderr, "usage: kf_dxprobe d3d11|d3d12 [--any-vendor] [--iters N]\n"); return 2; }
  say("start", "api=%s iters=%d any_vendor=%d", api, iters, any ? 1 : 0);
  std::string name;
  IDXGIAdapter1 *ad = pick_adapter(any, name);
  int rc = 1;
  if (ad) {
    say("selected", "\"%s\"", name.c_str());
    rc = !strcmp(api, "d3d11") ? run_d3d11(ad, iters) : run_d3d12(ad, iters);
    ad->Release();
  }
  if (rc == 0 && !g_fail) say("RESULT", "OK api=%s adapter=\"%s\"", api, name.c_str());
  else say("RESULT", "FAIL %s", g_why.c_str());
  return (rc == 0 && !g_fail) ? 0 : 1;
}
