/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Bounded SDR: RGB8888 -> DIRECT10 FP16 ILUT -> FP16 blend -> OCSC0 ->
 * DIRECT10 unsigned fixed-point OLUT -> RGB8888. LUT snapshots contain four
 * header entries and 1025 RGB/padding entries; no segmented or mirrored mode.
 * All buffer extents are checked by DisplayGpu before launching any kernel.
 */
#define DEV static __attribute__((device))
typedef unsigned int U;
typedef unsigned long long Q;
typedef unsigned short H;
typedef struct { int v[12]; } Matrix;

DEV float sat(float x) { return !(x > 0) ? 0 : (x < 1 ? x : 1); }
DEV float half_float(H bits) {
    float f;
    __asm__("cvt.f32.f16 %0, %1;" : "=f"(f) : "h"(bits));
    return f;
}
DEV float fp16(float f) {
    H h;
    __asm__("cvt.rn.f16.f32 %0, %1;" : "=h"(h) : "f"(f));
    return half_float(h);
}
DEV float lookup(const H *lut, float x, U channel, U interpolate, U input) {
    float pos = sat(x) * 1024.0f;
    U i = (U)pos;
    if (i > 1023) i = 1023;
    H a = lut[(i + 4) * 4 + channel];
    H b = lut[(i + 5) * 4 + channel];
    float fa = input ? half_float(a) : a / 65536.0f;
    float fb = input ? half_float(b) : b / 65536.0f;
    return interpolate ? fa + (fb - fa) * (pos - i) : fa;
}

/* Validate the actual immutable snapshot the kernels use, never a CPU reread.
 * Only SDR FP16 values [0,1] are accepted. Reserved padding/header are not sampled.
 * Invalid content sets a bounded status word; publication and completion stop.
 */
extern "C" __attribute__((global)) void kf_color_validate(const H *lut, U *status) {
    U i = __nvvm_read_ptx_sreg_ctaid_x() * 256u + __nvvm_read_ptx_sreg_tid_x();
    if (i < 1025u) {
        for (U c = 0; c < 3; ++c) {
            H h = lut[(i + 4) * 4 + c];
            if ((h & 0x8000u) || h > 0x3c00u)
                __nvvm_atom_or_gen_i((int *)status, 1);
        }
    }
}

extern "C" __attribute__((global)) void kf_color_compose(
    const U *src, float *dst, U layout, U pitch, U bh, U x0b, U y0,
    U width, U ox, U oy, U fw, U fh, U flags, int as, int bs, int ad, int bd,
    const H *lut, U interpolate) {
    U row = __nvvm_read_ptx_sreg_ctaid_x();
    for (U x = __nvvm_read_ptx_sreg_tid_x(); x < width; x += 256) {
        U dx = ox + x, dy = oy + row;
        if (dx >= fw || dy >= fh) continue;
        Q off;
        if (!layout) off = (Q)row * pitch + (Q)x * 4;
        else {
            Q xb = (Q)x0b + (Q)x * 4, y = (Q)y0 + row;
            Q gob = (((y >> (3 + bh)) * pitch + (xb >> 6)) << bh) + ((y >> 3) & ((1u << bh) - 1));
            off = gob * 512 + (xb & 15) + ((y & 3) << 4) + (((xb >> 4) & 1) << 6) + (((y >> 2) & 1) << 7) + (((xb >> 5) & 1) << 8);
        }
        U p = *(const U *)((const unsigned char *)src + off);
        U red = (p >> 16) & 255, green = (p >> 8) & 255, blue = p & 255;
        if (flags & 2) { U t = red; red = blue; blue = t; }
        /* UNORM8 -> UNORM10 bit replication. Default ILUT uses i/1023 FP16;
         * converting an 8-bit source by i/255 directly skips the hardware index.
         */
        U cs[3] = { red, green, blue };
        U alpha = (flags & 1) ? p >> 24 : 255;
        int fs = as + bs * (int)alpha / 255;
        int fd = ad + bd * (int)alpha / 255;
        fs = fs < 0 ? 0 : (fs > 255 ? 255 : fs);
        fd = fd < 0 ? 0 : (fd > 255 ? 255 : fd);
        Q d = ((Q)dy * fw + dx) * 4;
        for (U c = 0; c < 3; ++c) {
            U i = (cs[c] << 2) | (cs[c] >> 6);
            float v = lut ? lookup(lut, i / 1024.0f, c, interpolate, 1) : fp16(cs[c] / 255.0f);
            dst[d + c] = (flags & 4) ? v : fp16(v * (fs / 255.0f) + dst[d + c] * (fd / 255.0f));
        }
        dst[d + 3] = 1;
    }
}

extern "C" __attribute__((global)) void kf_color_output(
    const float *src, U *dst, U width, const H *lut, U interpolate, Matrix matrix) {
    U row = __nvvm_read_ptx_sreg_ctaid_x();
    for (U x = __nvvm_read_ptx_sreg_tid_x(); x < width; x += 256) {
        Q p = (Q)row * width + x;
        U packed = 0xff000000u;
        for (U c = 0; c < 3; ++c) {
            const int *m = matrix.v + c * 4;
            float v = src[p * 4] * (m[0] / 65536.0f) + src[p * 4 + 1] * (m[1] / 65536.0f) + src[p * 4 + 2] * (m[2] / 65536.0f) + m[3] / 65536.0f;
            v = lut ? lookup(lut, v, c, interpolate, 0) : sat(v);
            U q = (U)(sat(v) * 255.0f + 0.5f);
            packed |= q << (16 - 8 * c);
        }
        dst[p] = packed;
    }
}
