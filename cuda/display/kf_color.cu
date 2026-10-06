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
typedef struct {
    Matrix matrices[4];
    U segments[2][64];
    U entries[2][1025];
    U enable[2];
} Pipeline;


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
DEV float power2(int e) {
    union { U bits; float value; } f;
    f.bits = (U)(e + 127) << 23;
    return f.value;
}
DEV float fixed14(float x) {
    U q;
    float scaled = sat(x) * 16384.0f;
    __asm__("cvt.rni.u32.f32 %0, %1;" : "=r"(q) : "f"(scaled));
    return (q > 16383 ? 16383 : q) / 16384.0f;
}
DEV void matrix_apply(const Matrix *m, float *v, U fixed) {
    float tmp[3];
    for (U c=0; c<3; ++c) {
        const int *a = m->v + c*4;
        float x = v[0]*(a[0]/65536.0f) + v[1]*(a[1]/65536.0f) + v[2]*(a[2]/65536.0f) + a[3]/65536.0f;
        tmp[c] = fixed ? fixed14(x) : fp16(x);
    }
    for (U c=0; c<3; ++c) v[c] = tmp[c];
}
/* A segment stores log2(sample intervals), not log2(input step). CSC0 has
 * 33 logarithmic intervals [0,2^-25], [2^-25,2^-24], ..., [64,128];
 * CSC1 has 64 equal intervals over the 14-bit fixed-point domain.
 * Summed sample counts and initialization are bounded by the CPU planner.
 */
DEV float inline_lookup(const Pipeline *p, U stage, float x) {
    U base = 0, count = stage ? 64 : 33;
    for (U seg=0; seg<count; ++seg) {
        float low = stage ? seg / 64.0f : (seg ? power2((int)seg - 26) : 0);
        float high = stage ? (seg+1) / 64.0f : power2((int)seg - 25);
        U intervals = 1u << p->segments[stage][seg];
        if (x < high || seg == count-1) {
            float fraction = sat((x-low)/(high-low)) * intervals;
            U local = (U)fraction;
            if (local >= intervals) local = intervals-1;
            U a = p->entries[stage][base+local], b = p->entries[stage][base+local+1];
            float fa = stage ? half_float((H)a) : a / 65536.0f;
            float fb = stage ? half_float((H)b) : b / 65536.0f;
            return fa + (fb-fa)*(fraction-local);
        }
        base += intervals;
    }
    return 0;
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

/* The tone table has 64 equal input zones, each with a header-defined
 * power-of-two sample count. Every actual read is independently bounded even
 * when the validation kernel has rejected a hostile header.
 */
DEV float tone_lookup(const H *lut, float x, U interpolate) {
    float position = sat(x)*64.0f;
    U zone = (U)position;
    if (zone > 63) zone=63;
    U base=0;
    for (U seg=0; seg<zone; ++seg) {
        Q header=((const Q *)lut)[seg/16];
        base += 1u << ((header >> ((seg%16)*3)) & 7);
    }
    Q header=((const Q *)lut)[zone/16];
    U count=1u << ((header >> ((zone%16)*3)) & 7);
    float fraction=(position-zone)*count;
    U local=(U)fraction;
    if (local >= count) local=count-1;
    U index=base+local;
    if (index >= 1024) return 0;
    float a=lut[(index+4)*4+1]/65536.0f;
    float b=lut[(index+5)*4+1]/65536.0f;
    return interpolate ? a+(b-a)*(fraction-local) : a;
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

/* Validate the immutable tone snapshot: 64-zone linear VSS with bounded per-zone sample counts,
 * equal intensity channels and finite UNORM entries. Unsupported headers fail
 * before publication; they never select a guest-controlled read extent.
 */
extern "C" __attribute__((global)) void kf_tmo_validate(const H *lut, U *status) {
    U i = __nvvm_read_ptx_sreg_ctaid_x()*256u + __nvvm_read_ptx_sreg_tid_x();
    if (i == 0) {
        U samples=0;
        for (U seg=0; seg<64; ++seg) {
            Q header=((const Q *)lut)[seg/16];
            samples += 1u << ((header >> ((seg%16)*3)) & 7);
        }
        if (samples != 1024) __nvvm_atom_or_gen_i((int *)status, 2);
    }
    if (i < 1025u) {
        const H *entry = lut + (i+4)*4;
        if (entry[0] != entry[1] || entry[1] != entry[2])
            __nvvm_atom_or_gen_i((int *)status, 2);
    }
}

extern "C" __attribute__((global)) void kf_color_compose(
    const U *src, float *dst, U layout, U pitch, U bh, U x0b, U y0,
    U width, U ox, U oy, U fw, U fh, U flags, int as, int bs, int ad, int bd,
    const H *lut, U interpolate, const H *tmo, U tmo_interpolate, const Pipeline *pipeline, U *status) {
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
        /* GetLUTIndex() shifts UNORM8 left two bits. nvUnorm10ToFp16()
         * divides by 1024, so the default table returns component/256.
         */
        U cs[3] = { red, green, blue };
        U alpha = (flags & 1) ? p >> 24 : 255;
        int fs = as + bs * (int)alpha / 255;
        int fd = ad + bd * (int)alpha / 255;
        fs = fs < 0 ? 0 : (fs > 255 ? 255 : fs);
        fd = fd < 0 ? 0 : (fd > 255 ? 255 : fd);
        Q d = ((Q)dy * fw + dx) * 4;
        float v[3];
        for (U c=0; c<3; ++c) {
            U i = cs[c] << 2;
            v[c] = lut ? lookup(lut, i / 1024.0f, c, interpolate, 1) : fp16(cs[c]/255.0f);
        }
        if (pipeline) {
            matrix_apply(&pipeline->matrices[0], v, 0);
            if (pipeline->enable[0]) for (U c=0; c<3; ++c) v[c] = fixed14(inline_lookup(pipeline,0,v[c]));
            matrix_apply(&pipeline->matrices[1], v, pipeline->enable[0]);
        }
        /* Hardware orders components Ct, I, Cp. NO_CORRECTION keeps Ct/Cp.
         * A tone curve is NOT three independent RGB gamma lookups.
         */
        if (tmo) v[1] = tone_lookup(tmo, v[1], tmo_interpolate);
        if (pipeline) {
            matrix_apply(&pipeline->matrices[2], v, pipeline->enable[1]);
            if (pipeline->enable[1]) for (U c=0; c<3; ++c) v[c] = fp16(inline_lookup(pipeline,1,v[c]));
            matrix_apply(&pipeline->matrices[3], v, 0);
        }
        for (U c=0; c<3; ++c) {
            union { float f; U u; } bits;
            bits.f = v[c];
            if ((bits.u & 0x7fffffffu) >= 0x7f800000u)
                __nvvm_atom_or_gen_i((int *)status, 4);
            dst[d+c] = (flags & 4) ? v[c] : fp16(v[c]*(fs/255.0f) + dst[d+c]*(fd/255.0f));
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
            U q = (U)(sat(v) * 256.0f);
            if (q > 255) q = 255;
            packed |= q << (16 - 8 * c);
        }
        dst[p] = packed;
    }
}
