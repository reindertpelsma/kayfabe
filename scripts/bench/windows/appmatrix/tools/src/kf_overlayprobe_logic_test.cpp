// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// Host-side unit test of kf_overlayprobe_logic.h. Prints `OK <n> checks` and one JSON line; exit 0 iff all pass.
#include "kf_overlayprobe_logic.h"
#include <stdlib.h>
using namespace kfo;
static int n = 0, bad = 0;
#define CHECK(c) do { n++; if (!(c)) { bad++; printf("FAIL line %d: %s\n", __LINE__, #c); } } while (0)

int main() {
  // exact BT.709 limited-range values
  Yuv k = rgb709(0, 0, 0), w = rgb709(255, 255, 255), g = rgb709(128, 128, 128), r = rgb709(191, 0, 0), w75 = rgb709(191, 191, 191);
  CHECK(k.y == 16 && k.u == 128 && k.v == 128);
  CHECK(w.y == 235 && w.u == 128 && w.v == 128);
  CHECK(g.u == 128 && g.v == 128 && g.y == 126);
  CHECK(w75.y == 180 && w75.u == 128);
  CHECK(r.y == 51 && r.u == 109 && r.v == 212);
  Yuv none = rgb709(0, 0, 255);
  CHECK(none.y == 32 && none.u == 240 && none.v == 118);  // pure blue: Y=16+219*0.0722, Cb=240
  const Yuv *bt = bar_table();
  for (int i = 0; i < kBars; i++) { CHECK(bt[i].y >= 16 && bt[i].y <= 235 && bt[i].u >= 16 && bt[i].u <= 240 && bt[i].v >= 16 && bt[i].v <= 240); }
  for (int i = 0; i < kBars; i++) for (int j = i + 1; j < kBars; j++) CHECK(!(bt[i].y == bt[j].y && bt[i].u == bt[j].u && bt[i].v == bt[j].v));
  CHECK(bt[7].y == 16);  // last bar black

  // NV12 / P010 / YUY2 fill + decode round trip over many frames and sizes
  const int sizes[][2] = {{640, 360}, {1280, 720}, {854, 480}, {1920, 1080}};
  for (auto &s : sizes) {
    int W = s[0], H = s[1];
    if (W % 16) W = (W / 16) * 16;
    if (H % 2) H--;
    int pitch = W + 64;
    std::vector<uint8_t> nv((size_t)pitch * H * 3 / 2), pp((size_t)(pitch * 2) * H * 3 / 2), yy((size_t)(W * 2 + 32) * H);
    for (uint32_t f : {0u, 1u, 77u, 1000u, 65535u, 65536u + 5u, 123456u}) {
      uint32_t c; int mx;
      fill_nv12(nv.data(), pitch, W, H, f);
      CHECK(decode(W, H, [&](int x, int y) { return (int)nv[(size_t)y * pitch + x]; }, &c, &mx));
      CHECK(c == (f & 0xffff)); CHECK(mx == Layout(W, H).marker_x(f));
      // chroma of the bars: pixel in bar 5 (red) must carry its U/V in the interleaved plane
      int bx = W * 5 / 8 + 4; bx &= ~1;
      CHECK(nv[(size_t)(H + 2) * pitch + bx] == bt[5].u && nv[(size_t)(H + 2) * pitch + bx + 1] == bt[5].v);
      fill_p010(pp.data(), pitch * 2, W, H, f);
      CHECK(decode(W, H, [&](int x, int y) { return (int)(((uint16_t *)(pp.data() + (size_t)y * pitch * 2))[x] >> 8); }, &c, &mx));
      CHECK(c == (f & 0xffff)); CHECK(mx == Layout(W, H).marker_x(f));
      int ypitch = W * 2 + 32;
      fill_yuy2(yy.data(), ypitch, W, H, f);
      CHECK(decode(W, H, [&](int x, int y) { return (int)yy[(size_t)y * ypitch + x * 2]; }, &c, &mx));
      CHECK(c == (f & 0xffff)); CHECK(mx == Layout(W, H).marker_x(f));
      CHECK(yy[(size_t)2 * ypitch + (bx * 2) + 1] == bt[5].u && yy[(size_t)2 * ypitch + bx * 2 + 3] == bt[5].v);
    }
    // marker moves: consecutive frames differ in position (until wrap)
    CHECK(Layout(W, H).marker_x(1) == 8 && Layout(W, H).marker_x(2) == 16);
  }

  // tracker: overlay reached at 1.2 s, a graced 3 s demotion, an ungraced blip, recovery
  {
    Tracker t; t.allow(5000, 8000);
    for (int ms = 0; ms < 12000; ms += 33) {
      int m = ms < 1200 ? kComposed : kOverlay;
      if (ms >= 5000 && ms <= 7500) m = kNone;
      if (ms >= 10000 && ms < 10070) m = kComposed;
      t.add(ms, m);
    }
    Analysis a = t.analyze(12000);
    CHECK(a.first_overlay_ms >= 1200 && a.first_overlay_ms < 1300);
    CHECK(a.bad_outside_grace == 2);        // samples at 10032 and 10065 ms in the 70 ms blip
    CHECK(a.final_ok);
    CHECK(a.transitions.size() == 5);       // C->O, O->N, N->O, O->C, C->O
    CHECK(a.per_second.size() == 12);
    uint64_t sum = 0; for (int m = 0; m < kModes; m++) sum += a.totals[m]; CHECK(sum == a.samples);
    VerdictIn v; v.first_overlay_ms = a.first_overlay_ms; v.bad_outside = a.bad_outside_grace; v.final_ok = a.final_ok; v.frames = a.samples; v.expected = a.samples;
    CHECK(decide(v) == kLost);
    v.tolerate = 2; CHECK(decide(v) == kPass);
    v.first_overlay_ms = -1; CHECK(decide(v) == kNeverOverlay);
    v.stuck = true; CHECK(decide(v) == kStuck);
    v.device_lost = true; CHECK(decide(v) == kDeviceLost);
    VerdictIn s; s.first_overlay_ms = 10; s.final_ok = true; s.frames = 10; s.expected = 100; CHECK(decide(s) == kStarved);
  }
  {  // never overlay; and lost at the end
    Tracker t; for (int ms = 0; ms < 5000; ms += 33) t.add(ms, kComposed);
    Analysis a = t.analyze(5000); CHECK(a.first_overlay_ms < 0 && !a.final_ok);
    Tracker u; for (int ms = 0; ms < 6000; ms += 33) u.add(ms, ms < 3000 ? kOverlay : kComposed);
    Analysis b = u.analyze(6000); CHECK(b.first_overlay_ms == 0 && !b.final_ok && b.bad_outside_grace > 0);
  }

  // scenarios: sorted, positive grace, soak truncation, name lookup
  CHECK(scenario_id("steady") == 0 && scenario_id("soak") == 7 && scenario_id("5") == 4 && scenario_id("bogus") == -1);
  for (int id = 0; id < 8; id++) {
    Evs e; int64_t len = build_events(id, id == 7 ? 120000 : 30000, 1920, 1080, e);
    CHECK(len > 0);
    for (size_t i = 1; i < e.size(); i++) CHECK(e[i - 1].t <= e[i].t);
    for (auto &x : e) CHECK(x.t >= 0 && x.t < len);
    if (id == 7) { CHECK(!e.empty()); CHECK(e.back().t <= len - 3000 + 1); }
    if (id == 4) { CHECK(e.size() == 2 && e[0].k == EV_OCC_ON && e[0].grace >= 5000 && e[1].k == EV_OCC_OFF); }
    if (id == 6) { CHECK(e.size() == 10); int rc = 0; for (auto &x : e) rc += x.k == EV_SC_RECREATE; CHECK(rc == 5); }
  }
  {  // soak at 20 min stays bounded and ends in a steady tail
    Evs e; int64_t len = build_events(7, 1200000, 1920, 1080, e); CHECK(len == 1200000 && e.size() > 100 && e.back().t < len - 3000 + 1);
  }

  // JSON writer
  Json j; j.obj().kv("a", "q\"uote\\\n").kv("n", 5LL).kvd("d", 1.5).kvb("b", true).key("arr").arr().i64(1).i64(2).obj().kv("x", "y").end_obj().end_arr().key("e").obj().end_obj().end_obj();
  CHECK(j.text() == "{\"a\":\"q\\\"uote\\\\\\u000a\",\"n\":5,\"d\":1.500,\"b\":true,\"arr\":[1,2,{\"x\":\"y\"}],\"e\":{}}");

  printf("%s %d checks, %d failed\n", bad ? "FAILED" : "OK", n, bad);
  printf("%s\n", j.text().c_str());
  return bad ? 1 : 0;
}
