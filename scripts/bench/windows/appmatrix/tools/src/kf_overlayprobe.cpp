// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// kf_overlayprobe.cpp -- forces a hardware YUV overlay (multi-plane overlay, MPO) presentation and PROVES it.
//
//   kf_overlayprobe.exe [--scenario steady|move|resize|fullscreen|occlude|hide|recreate|soak | 1..8]
//        [--duration S] [--require-overlay-within S] [--fps N] [--format nv12|yuy2|p010] [--size WxH]
//        [--mode comp|hwnd] [--flags yuv|yuv+fsv|none] [--out FILE.json] [--tolerate N] [--grace-ms N]
//        [--ignore-support] [--any-vendor] [--list]
//
// A Win32 window, a D3D11 device on the NVIDIA adapter (WARP / Basic Render are refused; the adapter name is
// printed), and a flip-model YUV swap chain (default: CreateSwapChainForComposition + a DirectComposition visual,
// DXGI_SWAP_CHAIN_FLAG_YUV_VIDEO). Each frame is a known pattern (tools/src/kf_overlayprobe_logic.h: SMPTE-like
// 75% bars in exact BT.709 limited-range YUV, a marker that moves 8 px/frame, a 16-bit binary frame counter),
// written to a staging NV12/P010/YUY2 texture, copied into the back buffer and presented with Present1 at a fixed
// rate. PROOF: IDXGISwapChainMedia::GetFrameStatisticsMedia().CompositionMode == DXGI_FRAME_PRESENTATION_MODE_OVERLAY
// after each present; the per-second counts of each mode, every transition, the worst present latency and the
// IDXGIOutput3::CheckOverlaySupport flags go into one JSON result. If the overlay is not reached within
// --require-overlay-within seconds the program FAILS (exit 2); if reached and then lost outside the scenario's
// declared grace windows, or not overlay at the end, exit 3; a watchdog thread reports STUCK (exit 4) when no
// present completes for 3 s. Exit codes: 0 PASS, 2 NEVER_OVERLAY, 3 LOST, 4 STUCK, 5 SETUP (no NVIDIA adapter /
// device / overlay support), 6 USAGE, 7 TOO_FEW_FRAMES, 8 DEVICE_LOST.
// Output: line oriented `KFOVL <key> <value>`, then `KFOVL JSON {...}` and the last line
// `KFOVL RESULT PASS|<NAME> <why>`. Static mingw build: build_tools.sh.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <mmsystem.h>
#include <d3d11.h>
#include <dxgi1_3.h>
#include <dxgi1_4.h>
#include <dxgi1_6.h>
#include <dcomp.h>
#include <stdarg.h>
#include <atomic>
#include <algorithm>
#include "kf_overlayprobe_logic.h"

using namespace kfo;

// ------------------------------------------------------------------------------------------ globals
static const int64_t kStuckMs = 3000;
static CRITICAL_SECTION g_cs;                 // guards g_trk, g_evlog, g_stats
static Tracker g_trk;
static std::vector<std::pair<int64_t, std::string>> g_evlog;
static LARGE_INTEGER g_qpf;
static std::atomic<int64_t> g_beat{0};        // last heartbeat (ms since g_t0)
static std::atomic<const char *> g_phase{"init"};
static std::atomic<int64_t> g_t0{0};          // QPC ms at program start; reset when the scenario clock starts
static std::atomic<bool> g_finished{false};

struct Config {
  int scenario = 0; int64_t duration_ms = 30000, require_ms = 10000; int fps = 30; DXGI_FORMAT fmt = DXGI_FORMAT_NV12; const char *fmt_name = "nv12";
  int vw = 1280, vh = 720; bool hwnd_mode = false; UINT sc_flags = 0x200; const char *flags_name = "yuv"; std::string out; uint32_t tolerate = 0; int64_t grace_extra = 0;
  bool ignore_support = false, any_vendor = false;
} g_cfg;

struct Stats {
  std::string adapter; unsigned vendor = 0, device = 0; unsigned long long dedicated_mb = 0;
  std::string support_json;                   // {"nv12":{...},...}
  unsigned support_flags = 0; bool support_checked = false;
  uint64_t frames = 0, occluded = 0, stats_errors = 0; double worst_present_ms = 0, sum_present_ms = 0, worst_gap_ms = 0; long last_stats_hr = 0;
  std::string sc_flags_s; bool device_lost = false; long device_lost_hr = 0; uint32_t sc_creates = 0;
  std::string note;
} g_stats;

static int64_t now_ms() { LARGE_INTEGER c; QueryPerformanceCounter(&c); return (int64_t)(c.QuadPart * 1000 / g_qpf.QuadPart) - g_t0; }
static double now_ms_f() { LARGE_INTEGER c; QueryPerformanceCounter(&c); return (double)c.QuadPart * 1000.0 / (double)g_qpf.QuadPart; }
static void beat(const char *phase) { g_phase = phase; g_beat = now_ms(); }
static void say(const char *k, const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); printf("KFOVL %s ", k); vprintf(fmt, ap); printf("\n"); fflush(stdout); va_end(ap);
}
static void logev(const char *name) { EnterCriticalSection(&g_cs); g_evlog.push_back({now_ms(), name}); LeaveCriticalSection(&g_cs); say("event", "t=%lldms %s", (long long)now_ms(), name); }
template <class T> static void rel(T *&p) { if (p) { p->Release(); p = nullptr; } }

// ------------------------------------------------------------------------------------------ result
static int finish(int forced_code, const char *reason, int64_t end_ms, const char *stuck_phase = nullptr) {
  static std::atomic<bool> once{false};
  if (once.exchange(true)) { Sleep(5000); return forced_code; }   // the first caller (main or watchdog) writes the result and exits
  g_finished = true;                              // silences the watchdog while the result is written
  EnterCriticalSection(&g_cs);
  Analysis a = g_trk.analyze(end_ms);
  VerdictIn v; v.stuck = forced_code == kStuck; v.device_lost = g_stats.device_lost; v.first_overlay_ms = a.first_overlay_ms;
  v.bad_outside = a.bad_outside_grace; v.tolerate = g_cfg.tolerate; v.final_ok = a.final_ok; v.frames = g_stats.frames;
  v.expected = (uint64_t)(end_ms / 1000.0 * g_cfg.fps);
  int code = (forced_code == kSetup || forced_code == kUsage) ? forced_code : decide(v);
  std::string why = reason ? reason : "";
  if (!reason || !*reason) {
    switch (code) {
      case kPass: why = "overlay held"; break;
      case kNeverOverlay: why = "overlay never reached"; break;
      case kLost: why = "overlay lost (" + std::to_string(a.bad_outside_grace) + " non-overlay presents outside grace, final_ok=" + (a.final_ok ? "1" : "0") + ")"; break;
      case kStuck: why = "no present completed for 3 s"; break;
      case kStarved: why = "frames presented < 50% of expected"; break;
      case kDeviceLost: why = "device removed/reset"; break;
    }
  }
  Json j; j.obj();
  j.kv("program", "kf_overlayprobe").kv("version", 1LL).kv("scenario", scenario_names()[g_cfg.scenario]);
  j.kv("verdict", code == kPass ? "PASS" : "FAIL").kv("result", code_name(code)).kv("code", (long long)code).kv("reason", why);
  j.key("adapter").obj().kv("name", g_stats.adapter).kv("vendor", (long long)g_stats.vendor).kv("device", (long long)g_stats.device).kv("dedicated_mb", (long long)g_stats.dedicated_mb).end_obj();
  j.key("overlay_support").obj().kvb("checked", g_stats.support_checked).kv("chosen_format_flags", (long long)g_stats.support_flags)
      .kvb("direct", (g_stats.support_flags & 1) != 0).kvb("scaling", (g_stats.support_flags & 2) != 0).key("formats").obj();
  std::string head = j.text();
  std::string out = head + (g_stats.support_json.empty() ? std::string() : g_stats.support_json) + "}}";  // closes "formats" and "overlay_support"
  Json k;  // remaining fields: build as a fresh object and splice (drop its leading '{')
  k.obj();
  k.key("config").obj().kv("format", g_cfg.fmt_name).kv("mode", g_cfg.hwnd_mode ? "hwnd" : "comp").kv("flags", g_cfg.flags_name).kv("swapchain_flags", g_stats.sc_flags_s)
      .kv("video_w", (long long)g_cfg.vw).kv("video_h", (long long)g_cfg.vh).kv("fps", (long long)g_cfg.fps).kv("duration_ms", end_ms)
      .kv("require_overlay_within_ms", g_cfg.require_ms).kv("tolerate", (long long)g_cfg.tolerate).kv("grace_extra_ms", g_cfg.grace_extra).end_obj();
  k.kv("frames_presented", (long long)g_stats.frames).kv("expected_frames", (long long)v.expected).kv("occluded_presents", (long long)g_stats.occluded)
      .kv("stats_errors", (long long)g_stats.stats_errors).kv("last_stats_hr", (long long)(unsigned)g_stats.last_stats_hr).kv("swapchains_created", (long long)g_stats.sc_creates);
  k.kvd("worst_present_ms", g_stats.worst_present_ms).kvd("avg_present_ms", g_stats.frames ? g_stats.sum_present_ms / (double)g_stats.frames : 0).kvd("worst_frame_gap_ms", g_stats.worst_gap_ms);
  k.kv("first_overlay_ms", a.first_overlay_ms).kv("non_overlay_outside_grace", (long long)a.bad_outside_grace).kvb("final_overlay", a.final_ok);
  k.key("mode_totals").obj();
  for (int m = 0; m < kModes; m++) k.kv(mode_name(m), (long long)a.totals[m]);
  k.end_obj();
  k.key("per_second").arr();
  for (size_t s = 0; s < a.per_second.size(); s++) {
    k.obj().kv("s", (long long)s);
    for (int m = 0; m < kModes; m++) k.kv(mode_name(m), (long long)a.per_second[s][m]);
    k.end_obj();
  }
  k.end_arr();
  k.kv("transitions_total", (long long)a.transitions.size()).key("transitions").arr();
  for (size_t i = 0; i < a.transitions.size() && i < 300; i++) k.obj().kv("t_ms", (long long)a.transitions[i].t_ms).kv("from", mode_name(a.transitions[i].from)).kv("to", mode_name(a.transitions[i].to)).end_obj();
  k.end_arr();
  k.key("events").arr();
  for (auto &e : g_evlog) k.obj().kv("t_ms", (long long)e.first).kv("name", e.second).end_obj();
  k.end_arr();
  if (stuck_phase) k.key("stuck").obj().kv("phase", stuck_phase).end_obj();
  if (!g_stats.note.empty()) k.kv("note", g_stats.note);
  k.end_obj();
  std::string tail = k.text();                    // "{...}" -> ",..." spliced after the head
  out += "," + tail.substr(1);
  LeaveCriticalSection(&g_cs);
  if (!g_cfg.out.empty()) {
    FILE *f = fopen(g_cfg.out.c_str(), "wb");
    if (f) { fwrite(out.data(), 1, out.size(), f); fputc('\n', f); fclose(f); } else say("warn", "cannot write %s", g_cfg.out.c_str());
  }
  say("JSON", "%s", out.c_str());
  say("summary", "frames=%llu first_overlay_ms=%lld overlay=%llu composed=%llu none=%llu worst_present_ms=%.2f transitions=%zu", (unsigned long long)g_stats.frames,
      (long long)a.first_overlay_ms, (unsigned long long)a.totals[kOverlay], (unsigned long long)a.totals[kComposed], (unsigned long long)a.totals[kNone], g_stats.worst_present_ms, a.transitions.size());
  say("RESULT", "%s %s (code=%d)", code == kPass ? "PASS" : code_name(code), why.c_str(), code);
  return code;
}

static DWORD WINAPI watchdog(LPVOID) {
  for (;;) {
    Sleep(100);
    if (g_finished) return 0;
    int64_t t = now_ms();
    if (t - g_beat > kStuckMs) {
      const char *ph = g_phase.load();
      say("STUCK", "no heartbeat for %lld ms in phase '%s'", (long long)(t - g_beat), ph);
      int c = finish(kStuck, "", t, ph);
      fflush(stdout);
      ExitProcess((UINT)c);
    }
  }
}

// ------------------------------------------------------------------------------------------ D3D / DXGI state
static HWND g_hwnd = nullptr, g_occ = nullptr;
static ID3D11Device *g_dev = nullptr; static ID3D11DeviceContext *g_ctx = nullptr;
static IDXGIFactory2 *g_fac = nullptr; static IDXGIAdapter1 *g_adp = nullptr; static IDXGIDevice *g_dxgidev = nullptr;
static IDXGISwapChain1 *g_sc = nullptr; static IDXGISwapChainMedia *g_media = nullptr;
static ID3D11Texture2D *g_stage = nullptr;
static IDCompositionDevice *g_dc = nullptr; static IDCompositionTarget *g_target = nullptr; static IDCompositionVisual *g_visual = nullptr; static IDCompositionScaleTransform *g_scale = nullptr;
static int g_vw, g_vh;                          // current swap chain (video) size
static bool g_quit = false, g_fs = false; static RECT g_savedRect; static LONG_PTR g_savedStyle;
static RECT g_lastClient = {0, 0, 0, 0};
static const wchar_t kWndClass[] = L"KfOverlayProbe";

static LRESULT CALLBACK wndproc(HWND h, UINT m, WPARAM w, LPARAM l) {
  switch (m) {
    case WM_CLOSE: g_quit = true; return 0;
    case WM_ERASEBKGND: return 1;
    case WM_PAINT: { PAINTSTRUCT ps; BeginPaint(h, &ps); EndPaint(h, &ps); return 0; }
  }
  return DefWindowProcW(h, m, w, l);
}

static DXGI_FORMAT fmt_of(const char *s, const char **name) {
  if (!strcmp(s, "nv12")) { *name = "nv12"; return DXGI_FORMAT_NV12; }
  if (!strcmp(s, "yuy2")) { *name = "yuy2"; return DXGI_FORMAT_YUY2; }
  if (!strcmp(s, "p010")) { *name = "p010"; return DXGI_FORMAT_P010; }
  if (!strcmp(s, "bgra")) { *name = "bgra"; return DXGI_FORMAT_B8G8R8A8_UNORM; }   // CONTROL format: exercises the window/present/scenario machinery where no YUV swap chain can be made; never reaches an overlay
  return DXGI_FORMAT_UNKNOWN;
}

static bool pick_adapter() {
  if (FAILED(CreateDXGIFactory1(__uuidof(IDXGIFactory2), (void **)&g_fac))) { say("FAILED", "CreateDXGIFactory1"); return false; }
  IDXGIAdapter1 *a = nullptr;
  for (UINT i = 0; g_fac->EnumAdapters1(i, &a) == S_OK; i++) {
    DXGI_ADAPTER_DESC1 d; a->GetDesc1(&d);
    char name[256]; WideCharToMultiByte(CP_UTF8, 0, d.Description, -1, name, sizeof name, nullptr, nullptr);
    say("adapter_enum", "%u \"%s\" vendor=0x%04x flags=0x%x", i, name, d.VendorId, d.Flags);
    bool soft = (d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) || d.VendorId == 0x1414;          // WARP / Basic Render Driver
    if (!g_adp && !soft && (d.VendorId == 0x10DE || g_cfg.any_vendor)) {
      g_adp = a; g_stats.adapter = name; g_stats.vendor = d.VendorId; g_stats.device = d.DeviceId; g_stats.dedicated_mb = d.DedicatedVideoMemory >> 20;
    } else a->Release();
  }
  if (!g_adp) { say("FAILED", "no NVIDIA hardware adapter (WARP / Basic Render are refused)"); return false; }
  say("adapter", "\"%s\" vendor=0x%04x device=0x%04x dedicated_mb=%llu", g_stats.adapter.c_str(), g_stats.vendor, g_stats.device, g_stats.dedicated_mb);
  return true;
}

static bool check_overlay_support() {
  IDXGIOutput *o = nullptr;
  if (g_adp->EnumOutputs(0, &o) != S_OK) { say("FAILED", "adapter has no output (no display attached to the NVIDIA adapter)"); g_stats.note = "adapter has no output"; return false; }
  DXGI_OUTPUT_DESC od; o->GetDesc(&od);
  say("output", "left=%ld top=%ld right=%ld bottom=%ld attached=%d", od.DesktopCoordinates.left, od.DesktopCoordinates.top, od.DesktopCoordinates.right, od.DesktopCoordinates.bottom, od.AttachedToDesktop);
  IDXGIOutput3 *o3 = nullptr;
  if (FAILED(o->QueryInterface(__uuidof(IDXGIOutput3), (void **)&o3))) { say("FAILED", "IDXGIOutput3 missing (needs Windows 8.1+)"); o->Release(); g_stats.note = "no IDXGIOutput3"; return false; }
  static const struct { const char *n; DXGI_FORMAT f; } F[] = {{"nv12", DXGI_FORMAT_NV12}, {"yuy2", DXGI_FORMAT_YUY2}, {"p010", DXGI_FORMAT_P010}};
  Json j; bool first = true;
  for (auto &f : F) {
    UINT fl = 0; HRESULT hr = o3->CheckOverlaySupport(f.f, g_dev, &fl);
    say("overlay_support", "%s hr=0x%08lx flags=0x%x direct=%d scaling=%d", f.n, (unsigned long)hr, fl, (fl & 1) != 0, (fl & 2) != 0);
    if (f.f == g_cfg.fmt) g_stats.support_flags = SUCCEEDED(hr) ? fl : 0;
    Json e; e.obj().kv("hr", (long long)(unsigned)hr).kv("flags", (long long)fl).kvb("direct", (fl & 1) != 0).kvb("scaling", (fl & 2) != 0).end_obj();
    g_stats.support_json += (first ? "\"" : ",\"") + std::string(f.n) + "\":" + e.text(); first = false;
  }
  g_stats.support_checked = true;
  // Extra evidence (does not change the verdict): IDXGIOutput6::CheckHardwareCompositionSupport (bit0 FULLSCREEN, bit1 WINDOWED, bit2 CURSOR_STRETCHED),
  // the desktop colour-space/refresh info, the D3D11 format support of NV12/YUY2/P010 (bit 1<<? per D3D11_FORMAT_SUPPORT), and every other output's flags.
  IDXGIOutput6 *o6 = nullptr;
  if (SUCCEEDED(o->QueryInterface(__uuidof(IDXGIOutput6), (void **)&o6))) {
    UINT hf = 0; HRESULT hr6 = o6->CheckHardwareCompositionSupport(&hf);
    say("hwcomp", "IDXGIOutput6::CheckHardwareCompositionSupport hr=0x%08lx flags=0x%x fullscreen=%d windowed=%d cursor_stretched=%d", (unsigned long)hr6, hf, (hf & 1) != 0, (hf & 2) != 0, (hf & 4) != 0);
    char b[160]; snprintf(b, sizeof b, "IDXGIOutput6::CheckHardwareCompositionSupport hr=0x%08lx flags=0x%x", (unsigned long)hr6, hf); g_stats.note += std::string(g_stats.note.empty() ? "" : "; ") + b;
    o6->Release();
  } else say("hwcomp", "IDXGIOutput6 not available");
  for (auto &f : F) {
    UINT sup = 0; HRESULT hrs = g_dev->CheckFormatSupport(f.f, &sup);
    say("format_support", "%s hr=0x%08lx d3d11_format_support=0x%08x render_target=%d texture2d=%d display=%d", f.n, (unsigned long)hrs, sup, (sup & D3D11_FORMAT_SUPPORT_RENDER_TARGET) != 0, (sup & D3D11_FORMAT_SUPPORT_TEXTURE2D) != 0, (sup & D3D11_FORMAT_SUPPORT_DISPLAY) != 0);
  }
  o3->Release(); o->Release();
  for (UINT oi = 1;; oi++) {
    IDXGIOutput *ox = nullptr; if (g_adp->EnumOutputs(oi, &ox) != S_OK) break;
    DXGI_OUTPUT_DESC xd; ox->GetDesc(&xd); IDXGIOutput3 *x3 = nullptr;
    if (SUCCEEDED(ox->QueryInterface(__uuidof(IDXGIOutput3), (void **)&x3))) {
      for (auto &f : F) { UINT fl = 0; HRESULT hr = x3->CheckOverlaySupport(f.f, g_dev, &fl); say("overlay_support_output", "output=%u %s hr=0x%08lx flags=0x%x", oi, f.n, (unsigned long)hr, fl); }
      x3->Release();
    }
    say("output", "index=%u left=%ld top=%ld right=%ld bottom=%ld attached=%d", oi, xd.DesktopCoordinates.left, xd.DesktopCoordinates.top, xd.DesktopCoordinates.right, xd.DesktopCoordinates.bottom, xd.AttachedToDesktop);
    ox->Release();
  }
  return true;
}

static void update_scale() {
  if (!g_scale) return;
  RECT c; GetClientRect(g_hwnd, &c);
  int cw = c.right - c.left, ch = c.bottom - c.top;
  if (cw <= 0 || ch <= 0) return;
  if (cw == g_lastClient.right && ch == g_lastClient.bottom) return;
  g_lastClient.right = cw; g_lastClient.bottom = ch;
  g_scale->SetScaleX((float)cw / g_vw); g_scale->SetScaleY((float)ch / g_vh);
  g_dc->Commit();
}

static void drop_buffers() { rel(g_stage); g_ctx->ClearState(); g_ctx->Flush(); }

static bool make_stage() {
  D3D11_TEXTURE2D_DESC d = {};
  d.Width = g_vw; d.Height = g_vh; d.MipLevels = 1; d.ArraySize = 1; d.Format = g_cfg.fmt; d.SampleDesc.Count = 1; d.Usage = D3D11_USAGE_STAGING; d.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
  HRESULT hr = g_dev->CreateTexture2D(&d, nullptr, &g_stage);
  if (FAILED(hr)) { say("FAILED", "CreateTexture2D staging %s %dx%d hr=0x%08lx", g_cfg.fmt_name, g_vw, g_vh, (unsigned long)hr); return false; }
  return true;
}

static bool make_swapchain(int w, int h) {
  g_vw = w; g_vh = h;
  DXGI_SWAP_CHAIN_DESC1 d = {};
  d.Width = w; d.Height = h; d.Format = g_cfg.fmt; d.SampleDesc.Count = 1; d.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT; d.BufferCount = 2;
  d.Scaling = DXGI_SCALING_STRETCH; d.SwapEffect = DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL; d.AlphaMode = DXGI_ALPHA_MODE_IGNORE; d.Flags = g_cfg.sc_flags;
  char b[32]; snprintf(b, sizeof b, "0x%x", g_cfg.sc_flags); g_stats.sc_flags_s = b;
  HRESULT hr = g_cfg.hwnd_mode ? g_fac->CreateSwapChainForHwnd(g_dev, g_hwnd, &d, nullptr, nullptr, &g_sc) : g_fac->CreateSwapChainForComposition(g_dev, &d, nullptr, &g_sc);
  if (FAILED(hr)) { say("FAILED", "CreateSwapChainFor%s %s %dx%d flags=%s hr=0x%08lx", g_cfg.hwnd_mode ? "Hwnd" : "Composition", g_cfg.fmt_name, w, h, b, (unsigned long)hr); return false; }
  hr = g_sc->QueryInterface(__uuidof(IDXGISwapChainMedia), (void **)&g_media);
  if (FAILED(hr)) { say("FAILED", "IDXGISwapChainMedia missing hr=0x%08lx (needs Windows 8.1+)", (unsigned long)hr); return false; }
  if (!g_cfg.hwnd_mode) {
    hr = g_visual->SetContent(g_sc);
    if (FAILED(hr)) { say("FAILED", "IDCompositionVisual::SetContent hr=0x%08lx", (unsigned long)hr); return false; }
    g_lastClient = RECT{0, 0, 0, 0}; update_scale(); g_dc->Commit();
  }
  g_stats.sc_creates++;
  say("swapchain", "%s %dx%d flags=%s %s", g_cfg.fmt_name, w, h, b, g_cfg.hwnd_mode ? "hwnd" : "composition");
  return make_stage();
}

static bool init_dcomp() {
  if (g_cfg.hwnd_mode) return true;
  HRESULT hr = DCompositionCreateDevice(g_dxgidev, __uuidof(IDCompositionDevice), (void **)&g_dc);
  if (SUCCEEDED(hr)) hr = g_dc->CreateTargetForHwnd(g_hwnd, TRUE, &g_target);
  if (SUCCEEDED(hr)) hr = g_dc->CreateVisual(&g_visual);
  if (SUCCEEDED(hr)) hr = g_dc->CreateScaleTransform(&g_scale);
  if (SUCCEEDED(hr)) hr = g_visual->SetTransform(g_scale);
  if (SUCCEEDED(hr)) hr = g_target->SetRoot(g_visual);
  if (FAILED(hr)) { say("FAILED", "DirectComposition setup hr=0x%08lx", (unsigned long)hr); return false; }
  return true;
}

// ------------------------------------------------------------------------------------------ frame + present
static void sample_mode(int64_t t, int64_t *pcount) {
  DXGI_FRAME_STATISTICS_MEDIA st = {};
  HRESULT hr = g_media->GetFrameStatisticsMedia(&st);
  int mode;
  if (FAILED(hr)) { mode = kModeError; g_stats.stats_errors++; g_stats.last_stats_hr = (long)hr; } else { mode = mode_from_dxgi((int)st.CompositionMode); *pcount = st.PresentCount; }
  EnterCriticalSection(&g_cs); g_trk.add(t, mode); LeaveCriticalSection(&g_cs);
}

static bool present_frame(uint32_t frame, double *last_done_ms) {
  beat("fill");
  D3D11_MAPPED_SUBRESOURCE m;
  HRESULT hr = g_ctx->Map(g_stage, 0, D3D11_MAP_WRITE, 0, &m);
  if (FAILED(hr)) { say("FAILED", "Map staging hr=0x%08lx", (unsigned long)hr); g_stats.note = "Map staging failed"; return false; }
  switch (g_cfg.fmt) {
    case DXGI_FORMAT_NV12: fill_nv12((uint8_t *)m.pData, (int)m.RowPitch, g_vw, g_vh, frame); break;
    case DXGI_FORMAT_P010: fill_p010((uint8_t *)m.pData, (int)m.RowPitch, g_vw, g_vh, frame); break;
    case DXGI_FORMAT_B8G8R8A8_UNORM: {   // control pattern: horizontal gradient, a marker moving 8 px/frame, no overlay claim
      for (int y = 0; y < g_vh; y++) { uint32_t *row = (uint32_t *)((uint8_t *)m.pData + (size_t)y * m.RowPitch);
        for (int x = 0; x < g_vw; x++) { uint32_t g = (uint32_t)(x * 255 / g_vw); row[x] = 0xff000000u | (g << 16) | (g << 8) | (uint32_t)(y * 255 / g_vh); }
        int mx = (int)((frame * 8u) % (uint32_t)g_vw); for (int x = mx; x < mx + 16 && x < g_vw; x++) row[x] = 0xffffffffu; }
      break; }
    default: fill_yuy2((uint8_t *)m.pData, (int)m.RowPitch, g_vw, g_vh, frame); break;
  }
  g_ctx->Unmap(g_stage, 0);
  beat("copy");
  ID3D11Texture2D *bb = nullptr;
  hr = g_sc->GetBuffer(0, __uuidof(ID3D11Texture2D), (void **)&bb);
  if (FAILED(hr)) { say("FAILED", "GetBuffer hr=0x%08lx", (unsigned long)hr); return false; }
  g_ctx->CopyResource(bb, g_stage);
  bb->Release();
  beat("Present1");
  DXGI_PRESENT_PARAMETERS pp = {};
  double t0 = now_ms_f();
  hr = g_sc->Present1(1, 0, &pp);
  double t1 = now_ms_f();
  beat("post-present");
  double lat = t1 - t0;
  g_stats.worst_present_ms = std::max(g_stats.worst_present_ms, lat); g_stats.sum_present_ms += lat;
  if (*last_done_ms > 0) g_stats.worst_gap_ms = std::max(g_stats.worst_gap_ms, t1 - *last_done_ms);
  *last_done_ms = t1;
  if (hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET) {
    g_stats.device_lost = true; g_stats.device_lost_hr = (long)hr;
    say("FAILED", "Present1 device removed/reset hr=0x%08lx reason=0x%08lx", (unsigned long)hr, (unsigned long)g_dev->GetDeviceRemovedReason());
    return false;
  }
  if (hr == DXGI_STATUS_OCCLUDED) g_stats.occluded++;
  else if (FAILED(hr)) { say("warn", "Present1 hr=0x%08lx", (unsigned long)hr); g_stats.note = "Present1 failed"; }
  g_stats.frames++;
  int64_t pc = 0;
  sample_mode(now_ms(), &pc);
  return true;
}

// ------------------------------------------------------------------------------------------ scenario events
static void set_client_size(int cw, int ch) {
  RECT r = {0, 0, cw, ch}; AdjustWindowRect(&r, WS_OVERLAPPEDWINDOW, FALSE);
  SetWindowPos(g_hwnd, nullptr, 0, 0, r.right - r.left, r.bottom - r.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
}
static void set_fullscreen(bool on) {
  if (on == g_fs) return;
  if (on) {
    g_savedStyle = GetWindowLongPtrW(g_hwnd, GWL_STYLE); GetWindowRect(g_hwnd, &g_savedRect);
    MONITORINFO mi = {sizeof mi}; GetMonitorInfoW(MonitorFromWindow(g_hwnd, MONITOR_DEFAULTTONEAREST), &mi);
    SetWindowLongPtrW(g_hwnd, GWL_STYLE, WS_POPUP | WS_VISIBLE);
    SetWindowPos(g_hwnd, HWND_TOP, mi.rcMonitor.left, mi.rcMonitor.top, mi.rcMonitor.right - mi.rcMonitor.left, mi.rcMonitor.bottom - mi.rcMonitor.top, SWP_FRAMECHANGED | SWP_NOACTIVATE);
  } else {
    SetWindowLongPtrW(g_hwnd, GWL_STYLE, g_savedStyle);
    SetWindowPos(g_hwnd, HWND_NOTOPMOST, g_savedRect.left, g_savedRect.top, g_savedRect.right - g_savedRect.left, g_savedRect.bottom - g_savedRect.top, SWP_FRAMECHANGED | SWP_NOACTIVATE);
  }
  g_fs = on;
}
static void occluder(bool on) {
  if (!on) { if (g_occ) { DestroyWindow(g_occ); g_occ = nullptr; } return; }
  RECT r; GetWindowRect(g_hwnd, &r);
  int w = (r.right - r.left) / 3;
  g_occ = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, L"KfOverlayOccluder", L"occluder", WS_POPUP, r.left + w / 2, r.top, w, r.bottom - r.top, nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
  if (g_occ) ShowWindow(g_occ, SW_SHOWNOACTIVATE);
}
static bool recreate_sc(int w, int h, bool resize_only) {
  drop_buffers();
  if (resize_only) {
    HRESULT hr = g_sc->ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, g_cfg.sc_flags);
    if (FAILED(hr)) { say("FAILED", "ResizeBuffers %dx%d hr=0x%08lx", w, h, (unsigned long)hr); g_stats.note = "ResizeBuffers failed"; return false; }
    g_vw = w; g_vh = h; g_lastClient = RECT{0, 0, 0, 0}; update_scale();
    return make_stage();
  }
  rel(g_media); rel(g_sc);
  g_ctx->Flush();
  return make_swapchain(w, h);
}

static bool apply(const Ev &e) {
  beat("event"); logev(e.name);
  switch (e.k) {
    case EV_MOVE: SetWindowPos(g_hwnd, nullptr, e.a, e.b, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE); break;
    case EV_SIZE: set_client_size(e.a, e.b); break;
    case EV_MAX: ShowWindow(g_hwnd, SW_MAXIMIZE); break;
    case EV_RESTORE: ShowWindow(g_hwnd, SW_RESTORE); break;
    case EV_FS_ON: set_fullscreen(true); break;
    case EV_FS_OFF: set_fullscreen(false); break;
    case EV_OCC_ON: occluder(true); break;
    case EV_OCC_OFF: occluder(false); break;
    case EV_HIDE: ShowWindow(g_hwnd, SW_HIDE); break;
    case EV_SHOW: ShowWindow(g_hwnd, SW_SHOWNOACTIVATE); break;
    case EV_MIN: ShowWindow(g_hwnd, SW_MINIMIZE); break;
    case EV_UNMIN: ShowWindow(g_hwnd, SW_RESTORE); break;
    case EV_SC_RESIZE: return recreate_sc(e.a, e.b, true);
    case EV_SC_RECREATE: return recreate_sc(e.a, e.b, false);
  }
  return true;
}

static void pump() {
  MSG msg;
  while (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE)) { if (msg.message == WM_QUIT) g_quit = true; TranslateMessage(&msg); DispatchMessageW(&msg); }
}

// ------------------------------------------------------------------------------------------ main
static int usage(const char *why) {
  fprintf(stderr, "kf_overlayprobe: %s\nusage: kf_overlayprobe.exe [--scenario steady|move|resize|fullscreen|occlude|hide|recreate|soak|1..8] [--duration S] "
          "[--require-overlay-within S] [--fps N] [--format nv12|yuy2|p010] [--size WxH] [--mode comp|hwnd] [--flags yuv|yuv+fsv|none] [--out FILE] "
          "[--tolerate N] [--grace-ms N] [--ignore-support] [--any-vendor] [--list]\n", why);
  say("RESULT", "USAGE %s (code=6)", why);
  return kUsage;
}

int main(int argc, char **argv) {
  QueryPerformanceFrequency(&g_qpf);
  { LARGE_INTEGER c; QueryPerformanceCounter(&c); g_t0 = (int64_t)(c.QuadPart * 1000 / g_qpf.QuadPart); }
  InitializeCriticalSection(&g_cs);
  SetProcessDPIAware();
  int64_t user_dur = -1;
  for (int i = 1; i < argc; i++) {
    std::string a = argv[i];
    auto val = [&]() -> const char * { return i + 1 < argc ? argv[++i] : nullptr; };
    const char *v;
    if (a == "--list") { for (int s = 0; scenario_names()[s]; s++) printf("%d %s\n", s + 1, scenario_names()[s]); return 0; }
    else if (a == "--scenario" && (v = val())) { g_cfg.scenario = scenario_id(v); if (g_cfg.scenario < 0) return usage("unknown scenario"); }
    else if (a == "--duration" && (v = val())) user_dur = (int64_t)(atof(v) * 1000);
    else if (a == "--require-overlay-within" && (v = val())) g_cfg.require_ms = (int64_t)(atof(v) * 1000);
    else if (a == "--fps" && (v = val())) g_cfg.fps = std::max(1, std::min(120, atoi(v)));
    else if (a == "--format" && (v = val())) { g_cfg.fmt = fmt_of(v, &g_cfg.fmt_name); if (g_cfg.fmt == DXGI_FORMAT_UNKNOWN) return usage("unknown format"); }
    else if (a == "--size" && (v = val())) { if (sscanf(v, "%dx%d", &g_cfg.vw, &g_cfg.vh) != 2 || g_cfg.vw < 64 || g_cfg.vh < 64) return usage("bad --size"); }
    else if (a == "--mode" && (v = val())) { if (!strcmp(v, "hwnd")) g_cfg.hwnd_mode = true; else if (strcmp(v, "comp")) return usage("bad --mode"); }
    else if (a == "--flags" && (v = val())) { g_cfg.flags_name = v; g_cfg.sc_flags = !strcmp(v, "yuv") ? 0x200 : !strcmp(v, "yuv+fsv") ? 0x300 : !strcmp(v, "none") ? 0 : 0xffffffff; if (g_cfg.sc_flags == 0xffffffff) return usage("bad --flags"); }
    else if (a == "--out" && (v = val())) g_cfg.out = v;
    else if (a == "--tolerate" && (v = val())) g_cfg.tolerate = (uint32_t)atoi(v);
    else if (a == "--grace-ms" && (v = val())) g_cfg.grace_extra = atoll(v);
    else if (a == "--ignore-support") g_cfg.ignore_support = true;
    else if (a == "--any-vendor") g_cfg.any_vendor = true;
    else return usage(("bad argument " + a).c_str());
  }
  g_cfg.vw &= ~15; g_cfg.vh &= ~1;
  if (g_cfg.fmt == DXGI_FORMAT_B8G8R8A8_UNORM) {   // control run: no YUV flag, no overlay support gate, no "must reach overlay" abort (the verdict stays NEVER_OVERLAY by design)
    g_cfg.ignore_support = true; g_cfg.sc_flags = 0; g_cfg.flags_name = "none";
    g_cfg.require_ms = (user_dur > 0 ? user_dur : 600000) + 600000;
  }
  int sw = GetSystemMetrics(SM_CXSCREEN), sh = GetSystemMetrics(SM_CYSCREEN);
  int64_t def_dur = g_cfg.scenario == 7 ? 60000 : 30000;
  Evs evs; int64_t dur = build_events(g_cfg.scenario, user_dur > 0 ? user_dur : def_dur, sw, sh, evs);
  say("start", "scenario=%s duration_ms=%lld fps=%d format=%s mode=%s flags=%s video=%dx%d require_overlay_within_ms=%lld", scenario_names()[g_cfg.scenario], (long long)dur,
      g_cfg.fps, g_cfg.fmt_name, g_cfg.hwnd_mode ? "hwnd" : "comp", g_cfg.flags_name, g_cfg.vw, g_cfg.vh, (long long)g_cfg.require_ms);
  for (const Ev &e : evs) if (e.grace > 0) g_trk.allow(e.t, e.t + e.grace + g_cfg.grace_extra);
  g_stats.sc_flags_s = "0";

  HANDLE wd = CreateThread(nullptr, 0, watchdog, nullptr, 0, nullptr);
  if (wd) CloseHandle(wd);
  beat("setup");

  auto setup_fail = [&](const char *why) { say("setup", "%s", why); return finish(kSetup, why, now_ms()); };
  if (!pick_adapter()) return setup_fail("no NVIDIA hardware adapter");
  {
    D3D_FEATURE_LEVEL fls[] = {D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0}, got;
    HRESULT hr = D3D11CreateDevice(g_adp, D3D_DRIVER_TYPE_UNKNOWN, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, fls, 2, D3D11_SDK_VERSION, &g_dev, &got, &g_ctx);
    if (FAILED(hr)) { say("FAILED", "D3D11CreateDevice hr=0x%08lx", (unsigned long)hr); return setup_fail("D3D11CreateDevice failed"); }
    say("device", "feature_level=0x%x", (unsigned)got);
    g_dev->QueryInterface(__uuidof(IDXGIDevice), (void **)&g_dxgidev);
  }
  if (!check_overlay_support() && !g_cfg.ignore_support) return setup_fail("overlay support query failed");
  if (!(g_stats.support_flags & 1) && !g_cfg.ignore_support) return setup_fail("CheckOverlaySupport reports no DXGI_OVERLAY_SUPPORT_FLAG_DIRECT for the chosen format (use --ignore-support to try anyway)");

  WNDCLASSW wc = {}; wc.lpfnWndProc = wndproc; wc.hInstance = GetModuleHandleW(nullptr); wc.hCursor = LoadCursorW(nullptr, (LPCWSTR)IDC_ARROW); wc.lpszClassName = kWndClass;
  RegisterClassW(&wc);
  WNDCLASSW oc = {}; oc.lpfnWndProc = DefWindowProcW; oc.hInstance = wc.hInstance; oc.hbrBackground = CreateSolidBrush(RGB(200, 30, 200)); oc.lpszClassName = L"KfOverlayOccluder";
  RegisterClassW(&oc);
  int cw = std::min(960, sw - 100), ch = std::min(540, sh - 100);
  RECT r = {0, 0, cw, ch}; AdjustWindowRect(&r, WS_OVERLAPPEDWINDOW, FALSE);
  g_hwnd = CreateWindowExW(0, kWndClass, L"kf_overlayprobe", WS_OVERLAPPEDWINDOW, 50, 50, r.right - r.left, r.bottom - r.top, nullptr, nullptr, wc.hInstance, nullptr);
  if (!g_hwnd) return setup_fail("CreateWindow failed");
  ShowWindow(g_hwnd, SW_SHOW);
  beat("swapchain");
  if (!init_dcomp() || !make_swapchain(g_cfg.vw, g_cfg.vh)) return setup_fail("swap chain / DirectComposition setup failed (see FAILED line)");

  timeBeginPeriod(1);
  { LARGE_INTEGER c; QueryPerformanceCounter(&c); g_t0 = (int64_t)(c.QuadPart * 1000 / g_qpf.QuadPart); }   // scenario clock: t = 0 now
  beat("loop");
  const int64_t start = 0;
  const double frame_ms = 1000.0 / g_cfg.fps;
  double next = now_ms_f(), last_done = 0;
  uint32_t frame = 0; size_t ev_i = 0; int abort_code = 0; const char *abort_why = "";
  for (;;) {
    beat("loop");
    pump();
    int64_t t = now_ms() - start;
    if (g_quit) { abort_code = kSetup; abort_why = "window closed"; break; }
    if (t >= dur) break;
    if (t >= g_cfg.require_ms) {
      EnterCriticalSection(&g_cs); Analysis a = g_trk.analyze(t); LeaveCriticalSection(&g_cs);
      if (a.first_overlay_ms < 0) { abort_code = kNeverOverlay; abort_why = "overlay never reached within --require-overlay-within"; break; }
    }
    while (ev_i < evs.size() && evs[ev_i].t <= t) { if (!apply(evs[ev_i])) { abort_code = kSetup; abort_why = "scenario event failed"; break; } ev_i++; }
    if (abort_code) break;
    if (!g_cfg.hwnd_mode) update_scale();
    double n = now_ms_f();
    if (n < next) { Sleep(1); continue; }
    if (!present_frame(frame++, &last_done)) { abort_code = g_stats.device_lost ? kDeviceLost : kSetup; abort_why = g_stats.device_lost ? "device removed/reset" : "present path failed"; break; }
    next += frame_ms;
    if (now_ms_f() - next > 4 * frame_ms) next = now_ms_f();
  }
  int64_t end = now_ms();
  timeEndPeriod(1);
  occluder(false);
  int rc = finish(abort_code, abort_code ? abort_why : "", end);
  fflush(stdout);
  ExitProcess((UINT)rc);   // skip D3D teardown: a wedged driver must not hang the exit
}
