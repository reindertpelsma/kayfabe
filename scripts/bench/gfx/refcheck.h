// refcheck.h — a CPU reference for the three-triangle scene, shared by vk_gfx.c and egl_gfx.c.
//
// ★ It exists so a render can be graded WITHOUT a host to compare against. It checks, on a
// sampling grid, every pixel centre that is more than EDGE_PX from every triangle edge and whose
// covering triangles are not within ZTIE of each other in depth:
//   - the winning triangle's colour (depth test LESS ⇒ nearest wins), ±CTOL per channel;
//   - the depth value, ±ZTOL (D32 in Vulkan, D24S8 read back as float in GL);
//   - uncovered pixels are the clear colour at depth 1.0;
//   - the second pass (B) equals a bilinear sample of the ACTUAL pass-1 image, transposed.
// ⊘ Edge pixels are skipped by construction: rasterisation rules decide them, not this model.
// The host-hash comparison is the strict grade; this is the floor that holds without one.
#ifndef KF_GFX_REFCHECK_H
#define KF_GFX_REFCHECK_H
#include <math.h>
#include <stdint.h>
#include <stdio.h>

#define RW 256
#define RH 256
static const float RP[9][3] = {
    {-0.9f, -0.8f, 0.2f}, { 0.9f, -0.6f, 0.8f}, { 0.0f,  0.9f, 0.5f},
    {-0.9f, -0.6f, 0.8f}, { 0.9f, -0.8f, 0.2f}, { 0.0f,  0.8f, 0.5f},
    {-0.3f, -0.3f, 0.4f}, { 0.3f, -0.3f, 0.6f}, { 0.0f,  0.4f, 0.1f}};
static const float RC[3][3] = {{1.0f, 0.2f, 0.1f}, {0.1f, 1.0f, 0.2f}, {0.2f, 0.3f, 1.0f}};
static const float RCLEAR[3] = {0.1f, 0.2f, 0.3f};

// Pixel-space triangle coverage: returns 1 and the interpolated z if (px,py) is inside
// triangle t by more than `margin` pixels from every edge; sets *near_edge if within margin.
static int ref_tri(int t, float px, float py, float margin, float *z, int *near_edge) {
    float X[3], Y[3], Z[3];
    for (int i = 0; i < 3; i++) {
        X[i] = (RP[3 * t + i][0] + 1.0f) * 0.5f * RW;
        Y[i] = (RP[3 * t + i][1] + 1.0f) * 0.5f * RH;
        Z[i] = RP[3 * t + i][2];
    }
    float area = (X[1] - X[0]) * (Y[2] - Y[0]) - (X[2] - X[0]) * (Y[1] - Y[0]);
    float w[3]; int inside = 1;
    for (int i = 0; i < 3; i++) {
        int a = (i + 1) % 3, b = (i + 2) % 3;
        float e = (X[b] - X[a]) * (py - Y[a]) - (Y[b] - Y[a]) * (px - X[a]);
        float len = sqrtf((X[b] - X[a]) * (X[b] - X[a]) + (Y[b] - Y[a]) * (Y[b] - Y[a]));
        float d = e / len * (area > 0 ? 1.0f : -1.0f);   // signed distance, + inside
        if (fabsf(d) < margin) *near_edge = 1;
        if (d <= 0) inside = 0;
        w[i] = e / area;
    }
    *z = w[0] * Z[0] + w[1] * Z[1] + w[2] * Z[2];
    return inside;
}

// rgba: pass-1 colour, row r ↔ NDC y = (r+0.5)/RH*2-1 (both APIs, see the callers).
// depth: float per pixel, window z in [0,1].  blit: pass-2 colour; blue_add is 0.1 (Vulkan) or 0 (GL).
static int ref_check(const char *tag, const uint8_t *rgba, const float *depth, const uint8_t *blit, float blue_add) {
    const float EDGE_PX = 2.0f, ZTIE = 0.02f, ZTOL = 2e-4f; const int CTOL = 3, BTOL = 4;
    int checked = 0, bad = 0, zbad = 0, bbad = 0;
    for (int y = 1; y < RH; y += 3) for (int x = 1; x < RW; x += 3) {
        float px = x + 0.5f, py = y + 0.5f; int near = 0, win = -1; float wz = 2.0f, z2 = 2.0f;
        for (int t = 0; t < 3; t++) {
            float z; if (ref_tri(t, px, py, EDGE_PX, &z, &near)) {
                if (z < wz) { z2 = wz; wz = z; win = t; } else if (z < z2) z2 = z;
            }
        }
        if (near || (win >= 0 && z2 - wz < ZTIE)) continue;
        checked++;
        const uint8_t *p = rgba + 4 * (y * RW + x);
        float want[3], wantz = 1.0f;
        if (win < 0) { for (int c = 0; c < 3; c++) want[c] = RCLEAR[c]; }
        else { for (int c = 0; c < 3; c++) want[c] = RC[win][c] * (0.6f + 0.4f * wz); wantz = wz; }
        int ok = 1;
        for (int c = 0; c < 3; c++) if (abs((int)p[c] - (int)lrintf(want[c] * 255.0f)) > CTOL) ok = 0;
        if (!ok) { if (bad < 4) printf("%s_REF_BAD px=(%d,%d) got=%u,%u,%u want=%d,%d,%d win=%d\n", tag, x, y,
                   p[0], p[1], p[2], (int)lrintf(want[0] * 255), (int)lrintf(want[1] * 255), (int)lrintf(want[2] * 255), win); bad++; }
        float d = depth[y * RW + x];
        if (fabsf(d - wantz) > ZTOL) { if (zbad < 4) printf("%s_REF_ZBAD px=(%d,%d) got=%f want=%f\n", tag, x, y, d, wantz); zbad++; }
    }
    // pass 2: B(x,y) = bilinear(A, u=(y+.5)/H*0.97+0.01, v=(x+.5)/W*0.97+0.01) * (1,.5,1) + (0,0,blue_add)
    int bchecked = 0;
    for (int y = 5; y < RH - 5; y += 7) for (int x = 5; x < RW - 5; x += 7) {
        float u = ((y + 0.5f) / RH * 0.97f + 0.01f) * RW - 0.5f, v = ((x + 0.5f) / RW * 0.97f + 0.01f) * RH - 0.5f;
        int x0 = (int)floorf(u), y0 = (int)floorf(v); float fx = u - x0, fy = v - y0;
        const uint8_t *q = blit + 4 * (y * RW + x); int ok = 1;
        for (int c = 0; c < 3; c++) {
            float s = (1 - fx) * (1 - fy) * rgba[4 * (y0 * RW + x0) + c] + fx * (1 - fy) * rgba[4 * (y0 * RW + x0 + 1) + c]
                    + (1 - fx) * fy * rgba[4 * ((y0 + 1) * RW + x0) + c] + fx * fy * rgba[4 * ((y0 + 1) * RW + x0 + 1) + c];
            float m = (c == 1 ? 0.5f : 1.0f), a = (c == 2 ? blue_add * 255.0f : 0.0f);
            float want = s * m + a; if (want > 255) want = 255;
            if (fabsf(q[c] - want) > BTOL) ok = 0;
        }
        bchecked++;
        if (!ok) { if (bbad < 4) printf("%s_REF_BLIT_BAD px=(%d,%d) got=%u,%u,%u\n", tag, x, y, q[0], q[1], q[2]); bbad++; }
    }
    printf("%s_REF checked=%d colour_bad=%d depth_bad=%d blit_checked=%d blit_bad=%d\n", tag, checked, bad, zbad, bchecked, bbad);
    // ⊘ A check over zero pixels is not a pass (a_sweep_that_reports_zero_must_first_report_one).
    if (checked < 1000 || bchecked < 500) { printf("%s_REF_VACUOUS\n", tag); return 1; }
    return bad + zbad + bbad;
}
#endif
