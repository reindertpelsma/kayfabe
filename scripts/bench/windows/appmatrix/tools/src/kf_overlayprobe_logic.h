// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// kf_overlayprobe_logic.h -- the pure (no Windows, no D3D) part of kf_overlayprobe: the test pattern in exact
// BT.709 limited-range YUV (NV12 / P010 / YUY2), its decoder, the presentation-mode tracker, the verdict
// and exit codes, the scenario timelines and a tiny JSON writer. Host-compiled and unit-tested by
// tests/test_overlayprobe_logic.py (tools/src/kf_overlayprobe_logic_test.cpp).
#pragma once
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <math.h>
#include <string>
#include <vector>
#include <array>

namespace kfo {

// ------------------------------------------------------------------------------------------ YUV pattern
struct Yuv { uint8_t y, u, v; };

// 8-bit full-range sRGB-coded RGB -> BT.709 limited range (Y 16..235, Cb/Cr 16..240), rounded to nearest.
inline Yuv rgb709(int r, int g, int b) {
  const double R = r / 255.0, G = g / 255.0, B = b / 255.0;
  const double Y = 0.2126 * R + 0.7152 * G + 0.0722 * B;
  const double Cb = (B - Y) / (2.0 * (1.0 - 0.0722)), Cr = (R - Y) / (2.0 * (1.0 - 0.2126));
  return Yuv{(uint8_t)lround(16 + 219 * Y), (uint8_t)lround(128 + 224 * Cb), (uint8_t)lround(128 + 224 * Cr)};
}

// SMPTE-like 75% colour bars, left to right.
static const int kBars = 8;
inline const Yuv *bar_table() {
  static Yuv t[kBars];
  static bool init = false;
  if (!init) {
    static const int rgb[kBars][3] = {{191, 191, 191}, {191, 191, 0}, {0, 191, 191}, {0, 191, 0}, {191, 0, 191}, {191, 0, 0}, {0, 0, 191}, {0, 0, 0}};
    for (int i = 0; i < kBars; i++) t[i] = rgb709(rgb[i][0], rgb[i][1], rgb[i][2]);
    init = true;
  }
  return t;
}
static const Yuv kWhite = {235, 128, 128}, kBlack = {16, 128, 128};

// Frame layout (w, h multiples of 16): rows [0,bars_end) colour bars, [bars_end,track_end) a black track with a
// white marker block that moves 8 px per frame, [track_end,h) a 16-cell binary counter (MSB first, white = 1).
struct Layout {
  int w, h, bars_end, track_end, mw, cw;
  Layout(int w_, int h_) : w(w_), h(h_) {
    bars_end = ((h * 70) / 100) & ~1; track_end = ((h * 85) / 100) & ~1; mw = (w / 16) & ~1; cw = w / 16;
  }
  int marker_x(uint32_t frame) const { return (int)(((uint64_t)frame * 8) % (uint64_t)(w - mw)) & ~1; }
};

inline Yuv sample(const Layout &L, int x, int y, uint32_t frame) {
  if (y < L.bars_end) return bar_table()[x * kBars / L.w];
  if (y < L.track_end) { int mx = L.marker_x(frame); return (x >= mx && x < mx + L.mw) ? kWhite : kBlack; }
  int bit = 15 - x / L.cw;  // cell 0 is the MSB
  return ((frame >> bit) & 1) ? kWhite : kBlack;
}

// NV12: Y plane (pitch bytes/row, h rows) then interleaved UV (pitch bytes/row, h/2 rows).
inline void fill_nv12(uint8_t *base, int pitch, int w, int h, uint32_t frame) {
  Layout L(w, h);
  for (int y = 0; y < h; y++) {
    uint8_t *row = base + (size_t)y * pitch;
    for (int x = 0; x < w; x++) row[x] = sample(L, x, y, frame).y;
  }
  for (int y = 0; y < h; y += 2) {
    uint8_t *row = base + (size_t)(h + y / 2) * pitch;
    for (int x = 0; x < w; x += 2) { Yuv p = sample(L, x, y, frame); row[x] = p.u; row[x + 1] = p.v; }
  }
}
// P010: same planes, 16-bit little-endian samples holding the 8-bit value in the top bits (v << 8).
inline void fill_p010(uint8_t *base, int pitch, int w, int h, uint32_t frame) {
  Layout L(w, h);
  for (int y = 0; y < h; y++) {
    uint16_t *row = (uint16_t *)(base + (size_t)y * pitch);
    for (int x = 0; x < w; x++) row[x] = (uint16_t)(sample(L, x, y, frame).y << 8);
  }
  for (int y = 0; y < h; y += 2) {
    uint16_t *row = (uint16_t *)(base + (size_t)(h + y / 2) * pitch);
    for (int x = 0; x < w; x += 2) { Yuv p = sample(L, x, y, frame); row[x] = (uint16_t)(p.u << 8); row[x + 1] = (uint16_t)(p.v << 8); }
  }
}
// YUY2: packed Y0 U Y1 V.
inline void fill_yuy2(uint8_t *base, int pitch, int w, int h, uint32_t frame) {
  Layout L(w, h);
  for (int y = 0; y < h; y++) {
    uint8_t *row = base + (size_t)y * pitch;
    for (int x = 0; x < w; x += 2) {
      Yuv a = sample(L, x, y, frame), b = sample(L, x + 1, y, frame);
      row[x * 2] = a.y; row[x * 2 + 1] = a.u; row[x * 2 + 2] = b.y; row[x * 2 + 3] = a.v;
    }
  }
}

// Decode the frame counter (low 16 bits) and the marker position back from the luma plane. getY(x,y) -> 0..255.
template <class GetY> inline bool decode(int w, int h, GetY getY, uint32_t *counter, int *marker_x) {
  Layout L(w, h);
  uint32_t c = 0;
  int cy = (L.track_end + h) / 2;
  for (int i = 0; i < 16; i++) { int v = getY(i * L.cw + L.cw / 2, cy); c = (c << 1) | (v > 126 ? 1u : 0u); }
  int ty = (L.bars_end + L.track_end) / 2, mx = -1;
  for (int x = 0; x < w; x++) if (getY(x, ty) > 126) { mx = x; break; }
  *counter = c; *marker_x = mx;
  return mx >= 0;
}

// ------------------------------------------------------------------------------------------ modes, tracker
// DXGI_FRAME_PRESENTATION_MODE values, plus kModeError for "GetFrameStatisticsMedia failed".
enum Mode { kComposed = 0, kOverlay = 1, kNone = 2, kCompFailure = 3, kModeError = 4, kModes = 5 };
inline const char *mode_name(int m) {
  static const char *n[kModes] = {"composed", "overlay", "none", "composition_failure", "stats_error"};
  return (m >= 0 && m < kModes) ? n[m] : "?";
}
inline Mode mode_from_dxgi(int v) { return (v >= 0 && v <= 3) ? (Mode)v : kModeError; }

struct Transition { int32_t t_ms; uint8_t from, to; };
struct Analysis {
  int64_t first_overlay_ms = -1;
  uint32_t bad_outside_grace = 0;   // non-overlay samples after first overlay and outside every grace window
  bool final_ok = false;            // >= 80% overlay in the last 2 s
  uint64_t totals[kModes] = {};
  std::vector<std::array<uint32_t, kModes>> per_second;
  std::vector<Transition> transitions;
  uint64_t samples = 0;
};

class Tracker {
 public:
  void add(int64_t t_ms, int mode) { s_.push_back({(int32_t)t_ms, (uint8_t)mode}); }
  void allow(int64_t a_ms, int64_t b_ms) { g_.push_back({a_ms, b_ms}); }  // demotion tolerated in [a,b]
  Analysis analyze(int64_t end_ms) const {
    Analysis a; a.samples = s_.size();
    int last = -1; int fin_n = 0, fin_ov = 0;
    for (const Sample &x : s_) {
      a.totals[x.mode]++;
      size_t sec = (size_t)(x.t < 0 ? 0 : x.t / 1000);
      if (a.per_second.size() <= sec) a.per_second.resize(sec + 1, std::array<uint32_t, kModes>{});
      a.per_second[sec][x.mode]++;
      if (last >= 0 && last != x.mode) a.transitions.push_back({x.t, (uint8_t)last, x.mode});
      last = x.mode;
      if (x.mode == kOverlay && a.first_overlay_ms < 0) a.first_overlay_ms = x.t;
      if (a.first_overlay_ms >= 0 && x.mode != kOverlay && !graced(x.t)) a.bad_outside_grace++;
      if (x.t >= end_ms - 2000) { fin_n++; if (x.mode == kOverlay) fin_ov++; }
    }
    a.final_ok = fin_n > 0 && fin_ov * 5 >= fin_n * 4;
    return a;
  }
 private:
  struct Sample { int32_t t; uint8_t mode; };
  struct Grace { int64_t a, b; };
  bool graced(int64_t t) const { for (const Grace &g : g_) if (t >= g.a && t <= g.b) return true; return false; }
  std::vector<Sample> s_;
  std::vector<Grace> g_;
};

// ------------------------------------------------------------------------------------------ verdict
enum Code { kPass = 0, kNeverOverlay = 2, kLost = 3, kStuck = 4, kSetup = 5, kUsage = 6, kStarved = 7, kDeviceLost = 8 };
inline const char *code_name(int c) {
  switch (c) {
    case kPass: return "PASS"; case kNeverOverlay: return "NEVER_OVERLAY"; case kLost: return "LOST"; case kStuck: return "STUCK";
    case kSetup: return "SETUP"; case kUsage: return "USAGE"; case kStarved: return "TOO_FEW_FRAMES"; case kDeviceLost: return "DEVICE_LOST";
  }
  return "?";
}
struct VerdictIn { bool stuck = false, device_lost = false; int64_t first_overlay_ms = -1; uint32_t bad_outside = 0, tolerate = 0; bool final_ok = false; uint64_t frames = 0, expected = 0; };
inline int decide(const VerdictIn &v) {
  if (v.device_lost) return kDeviceLost;
  if (v.stuck) return kStuck;
  if (v.first_overlay_ms < 0) return kNeverOverlay;
  if (v.bad_outside > v.tolerate || !v.final_ok) return kLost;
  if (v.frames * 2 < v.expected) return kStarved;
  return kPass;
}

// ------------------------------------------------------------------------------------------ scenarios
enum EvKind { EV_MOVE, EV_SIZE, EV_MAX, EV_RESTORE, EV_FS_ON, EV_FS_OFF, EV_OCC_ON, EV_OCC_OFF, EV_HIDE, EV_SHOW, EV_MIN, EV_UNMIN, EV_SC_RESIZE, EV_SC_RECREATE };
struct Ev { int64_t t; EvKind k; int a, b; int64_t grace; const char *name; };
inline const char *const *scenario_names() {
  static const char *n[] = {"steady", "move", "resize", "fullscreen", "occlude", "hide", "recreate", "soak", nullptr};
  return n;
}
// accepts a name or 1..8
inline int scenario_id(const char *s) {
  const char *const *n = scenario_names();
  for (int i = 0; n[i]; i++) if (!strcmp(s, n[i])) return i;
  if (s[0] >= '1' && s[0] <= '8' && !s[1]) return s[0] - '1';
  return -1;
}
typedef std::vector<Ev> Evs;
inline int64_t tri(int64_t i, int64_t n) { i %= 2 * n; return i < n ? i : 2 * n - i; }  // 0..n..0

inline int64_t gen_move(Evs &e, int64_t t0, int sw, int sh) {
  int xmax = sw > 1100 ? sw - 1100 : 0, ymax = sh > 650 ? sh - 650 : 0;
  for (int i = 0; i < 200; i++) e.push_back({t0 + 1000 + i * 100, EV_MOVE, (int)(xmax * tri(i, 100) / 100), (int)(ymax * tri(i * 2, 100) / 100), 0, "move"});
  return t0 + 1000 + 200 * 100 + 1000;
}
inline int64_t gen_resize(Evs &e, int64_t t0) {
  static const int sz[][2] = {{1280, 720}, {640, 360}, {960, 540}};
  int64_t t = t0 + 2000;
  for (auto &s : sz) { e.push_back({t, EV_SIZE, s[0], s[1], 800, "size"}); t += 2500; }
  e.push_back({t, EV_MAX, 0, 0, 1500, "maximize"}); t += 3000;
  e.push_back({t, EV_RESTORE, 0, 0, 1500, "restore"}); t += 3000;
  return t;
}
inline int64_t gen_fullscreen(Evs &e, int64_t t0) {
  int64_t t = t0 + 2500;
  for (int i = 0; i < 3; i++) { e.push_back({t, EV_FS_ON, 0, 0, 2000, "fullscreen_on"}); t += 4000; e.push_back({t, EV_FS_OFF, 0, 0, 2000, "fullscreen_off"}); t += 3000; }
  return t;
}
inline int64_t gen_occlude(Evs &e, int64_t t0) {
  e.push_back({t0 + 3000, EV_OCC_ON, 0, 0, 7000, "occluder_open"});    // covers the 5 s window plus 2 s recovery
  e.push_back({t0 + 8000, EV_OCC_OFF, 0, 0, 2000, "occluder_close"});
  return t0 + 13000;
}
inline int64_t gen_hide(Evs &e, int64_t t0) {
  e.push_back({t0 + 3000, EV_HIDE, 0, 0, 4500, "hide"});
  e.push_back({t0 + 5000, EV_SHOW, 0, 0, 2500, "show"});
  e.push_back({t0 + 8000, EV_MIN, 0, 0, 4500, "minimize"});
  e.push_back({t0 + 10000, EV_UNMIN, 0, 0, 2500, "unminimize"});
  return t0 + 14000;
}
inline int64_t gen_recreate(Evs &e, int64_t t0) {
  static const int sz[][2] = {{640, 360}, {1280, 720}, {854, 480}, {1920, 1080}, {960, 544}};
  for (int k = 0; k < 10; k++) {
    const int *s = sz[k % 5];
    e.push_back({t0 + 2000 * (k + 1), (k & 1) ? EV_SC_RECREATE : EV_SC_RESIZE, s[0], s[1], 1500, (k & 1) ? "sc_recreate" : "sc_resize"});
  }
  return t0 + 2000 * 11;
}
// Fills `e` (sorted by time) and returns the scenario's length in ms. user_dur_ms applies to steady and soak only.
inline int64_t build_events(int id, int64_t user_dur_ms, int sw, int sh, Evs &e) {
  e.clear();
  int64_t end = 0;
  switch (id) {
    case 0: end = user_dur_ms; break;
    case 1: end = gen_move(e, 0, sw, sh); break;
    case 2: end = gen_resize(e, 0); break;
    case 3: end = gen_fullscreen(e, 0); break;
    case 4: end = gen_occlude(e, 0); break;
    case 5: end = gen_hide(e, 0); break;
    case 6: end = gen_recreate(e, 0); break;
    case 7: {  // soak: whole sub-scenarios in rotation while they still fit with 3 s of steady tail
      end = user_dur_ms;
      int64_t t = 2000; int k = 0;
      for (;; k++) {
        Evs tmp; int64_t len = 0;
        switch (k % 6) {
          case 0: len = gen_move(tmp, 0, sw, sh); break; case 1: len = gen_resize(tmp, 0); break; case 2: len = gen_fullscreen(tmp, 0); break;
          case 3: len = gen_occlude(tmp, 0); break; case 4: len = gen_hide(tmp, 0); break; default: len = gen_recreate(tmp, 0); break;
        }
        if (t + len > end - 3000) break;
        for (Ev x : tmp) { x.t += t; e.push_back(x); }
        t += len;
      }
      break;
    }
  }
  return end;
}

// ------------------------------------------------------------------------------------------ JSON writer
class Json {
 public:
  Json &obj() { sep(); s_ += '{'; st_.push_back(true); return *this; }
  Json &end_obj() { s_ += '}'; st_.pop_back(); return *this; }
  Json &arr() { sep(); s_ += '['; st_.push_back(true); return *this; }
  Json &end_arr() { s_ += ']'; st_.pop_back(); return *this; }
  Json &key(const char *k) { sep(); quote(k); s_ += ':'; after_key_ = true; return *this; }
  Json &str(const std::string &v) { sep(); quote(v.c_str()); return *this; }
  Json &num(double v) { sep(); char b[40]; snprintf(b, sizeof b, "%.3f", v); s_ += b; return *this; }
  Json &i64(long long v) { sep(); s_ += std::to_string(v); return *this; }
  Json &boolean(bool v) { sep(); s_ += v ? "true" : "false"; return *this; }
  Json &kv(const char *k, const std::string &v) { return key(k).str(v); }
  Json &kv(const char *k, const char *v) { return key(k).str(v); }
  Json &kv(const char *k, long long v) { return key(k).i64(v); }
  Json &kvd(const char *k, double v) { return key(k).num(v); }
  Json &kvb(const char *k, bool v) { return key(k).boolean(v); }
  const std::string &text() const { return s_; }
 private:
  void sep() { if (after_key_) { after_key_ = false; return; } if (!st_.empty()) { if (!st_.back()) s_ += ','; st_.back() = false; } }
  void quote(const char *v) {
    s_ += '"';
    for (const unsigned char *p = (const unsigned char *)v; *p; p++) {
      if (*p == '"' || *p == '\\') { s_ += '\\'; s_ += (char)*p; }
      else if (*p < 0x20) { char b[8]; snprintf(b, sizeof b, "\\u%04x", *p); s_ += b; }
      else s_ += (char)*p;
    }
    s_ += '"';
  }
  std::string s_; std::vector<bool> st_; bool after_key_ = false;
};

}  // namespace kfo
