/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * kf_yuv.cu - the console copy's YUV window kernel: one window whose SET_PARAMS.FORMAT is a YUV
 * format the console composes (Edge's video overlay plane, 2026-10-10; the Linux plane oracle's
 * NV12/P010/YUYV planes, 2026-10-14), converted to RGB and written OPAQUE into the compose kernel's
 * XRGB8888 staging frame (or the SDR pipeline's FP32 frame).
 *
 * Layouts (`layout`):
 *   0  semi-planar, 8-bit samples:  plane 0 Y8, plane 1 interleaved chroma byte pairs
 *      (Y8___U8V8_N444 / _N422, Y8___V8U8_N420)
 *   1  semi-planar, 16-bit little-endian containers, data MSB-aligned (Y10___*, Y12___*: P010 ...)
 *   2  packed 4:2:2 Y0 C0 Y1 C1 (Y8_U8__Y8_V8_N422, YUY2)      - one plane
 *   3  packed 4:2:2 C0 Y0 C1 Y1 (U8_Y8__V8_Y8_N422, UYVY)      - one plane
 * Every sample is taken as a 16-bit value (8-bit samples << 8) = value / 65536 of full scale.
 *
 * Colour: the window's OWN 3x4 FMT matrix (SET_FMT_COEFFICIENT_C00..C23, S4.16, rows R/G/B over the
 * input channels V(Cr), Y, U(Cb), 1 - nvkms-evo3.c EvoSetFMTMatrixC5), integer arithmetic only:
 *   out = clamp((sum(c * s) + (c3 << 16)) * 255 / 2^32), rounded.
 * Scaling (`filter`): 0 = nearest (the source pixel x*sw/dw), 1 = bilinear on pixel centres, luma and
 * chroma (chroma sited at the centre of its luma block), clamped to the source rectangle.
 *
 * Bounds are the HOST's job (kf_disp::scanout::plan_yuv, kf_cuda::display::DisplayGpu::compose_yuv):
 * every address formed here lies inside the extents the plan checked against the context DMAs and
 * the store - every sample, nearest or bilinear, is inside the source rectangle - and every write
 * inside the fw x fh frame. The CPU reference is kf_disp::scanout::yuv_reference; kf-disp's
 * tests/yuv_kernel.rs runs THIS body on the host (-DKF_HOST) over every (CTA, thread) and compares
 * byte for byte.
 *
 * ONE SOURCE, TWO COMPILERS (cuda/display/make_yuv_ptx.sh for the device). Plain C only below.
 */
#ifdef KF_HOST
#define KF_DEVICE static
#else
#define KF_DEVICE static __attribute__((device))
#endif

#define KF_YUV_THREADS 256u

typedef struct {
	const unsigned char *ys, *cs;
	unsigned int bl, yp, cp, bh;
	unsigned int sx0, sy0, sw, sh;
	unsigned int ox, oy, dw, dh, fw, fh;
	unsigned int sxl, syl, vu, layout, filter;
	int m[12];
} kf_yuv_args;

KF_DEVICE unsigned long long kf_addr(unsigned int bl, unsigned long long xb, unsigned long long y,
				      unsigned int pitch, unsigned int bh)
{
	if (!bl)
		return y * pitch + xb;
	unsigned long long gob = (((y >> (3u + bh)) * pitch + (xb >> 6)) << bh) +
				 ((y >> 3) & ((1ull << bh) - 1ull));
	return gob * 512ull + (xb & 15ull) + ((y & 3ull) << 4) + (((xb >> 4) & 1ull) << 6) +
	       (((y >> 2) & 1ull) << 7) + (((xb >> 5) & 1ull) << 8);
}

/* one 16-bit sample at byte column xb of row y */
KF_DEVICE unsigned int kf_s(const unsigned char *p, const kf_yuv_args *a, unsigned long long xb,
			    unsigned long long y, unsigned int pitch, unsigned int wide)
{
	unsigned long long o = kf_addr(a->bl, xb, y, pitch, a->bh);
	if (!wide)
		return (unsigned int)p[o] << 8;
	/* a 16-bit container never straddles a 16-byte GOB sector: xb is even */
	return (unsigned int)p[o] | ((unsigned int)p[o + 1ull] << 8);
}

KF_DEVICE unsigned int kf_luma(const kf_yuv_args *a, unsigned long long x, unsigned long long y)
{
	switch (a->layout) {
	case 1u:
		return kf_s(a->ys, a, x * 2ull, y, a->yp, 1u);
	case 2u:
		return kf_s(a->ys, a, x * 2ull, y, a->yp, 0u);
	case 3u:
		return kf_s(a->ys, a, x * 2ull + 1ull, y, a->yp, 0u);
	default:
		return kf_s(a->ys, a, x, y, a->yp, 0u);
	}
}

/* chroma pair (c0, c1) in the format's storage order at chroma column cx, chroma row cy */
KF_DEVICE void kf_chroma(const kf_yuv_args *a, unsigned long long cx, unsigned long long cy,
			 unsigned int *c0, unsigned int *c1)
{
	switch (a->layout) {
	case 1u:
		*c0 = kf_s(a->cs, a, cx * 4ull, cy, a->cp, 1u);
		*c1 = kf_s(a->cs, a, cx * 4ull + 2ull, cy, a->cp, 1u);
		return;
	case 2u:
		*c0 = kf_s(a->ys, a, cx * 4ull + 1ull, cy, a->yp, 0u);
		*c1 = kf_s(a->ys, a, cx * 4ull + 3ull, cy, a->yp, 0u);
		return;
	case 3u:
		*c0 = kf_s(a->ys, a, cx * 4ull, cy, a->yp, 0u);
		*c1 = kf_s(a->ys, a, cx * 4ull + 2ull, cy, a->yp, 0u);
		return;
	default:
		*c0 = kf_s(a->cs, a, cx * 2ull, cy, a->cp, 0u);
		*c1 = kf_s(a->cs, a, cx * 2ull + 1ull, cy, a->cp, 0u);
		return;
	}
}

/* one output channel of the FMT matrix row r: the 8-bit value */
KF_DEVICE unsigned int kf_ch(const int *m, unsigned int r, unsigned int y, unsigned int u,
			     unsigned int v)
{
	long long acc = (long long)m[r * 4u] * (long long)v + (long long)m[r * 4u + 1u] * (long long)y +
			(long long)m[r * 4u + 2u] * (long long)u + (long long)m[r * 4u + 3u] * 65536ll;
	long long q = acc * 255ll + 2147483648ll;
	if (q <= 0)
		return 0u;
	q >>= 32;
	return q > 255 ? 255u : (unsigned int)q;
}

KF_DEVICE unsigned int kf_convert(const int *m, unsigned int y, unsigned int u, unsigned int v)
{
	return 0xff000000u | (kf_ch(m, 0u, y, u, v) << 16) | (kf_ch(m, 1u, y, u, v) << 8) |
	       kf_ch(m, 2u, y, u, v);
}

/* bilinear position on one axis: destination index d of n destination pixels over a source run
 * [s0, s0 + len) -> absolute source position in 16.16, centres mapped, clamped to the run */
KF_DEVICE long long kf_pos(unsigned int d, unsigned int n, unsigned int s0, unsigned int len)
{
	long long p = ((2ll * d + 1ll) * (long long)len * 65536ll) / (2ll * (long long)n) - 32768ll;
	long long hi = ((long long)len - 1ll) * 65536ll;
	if (p < 0)
		p = 0;
	if (p > hi)
		p = hi;
	return (long long)s0 * 65536ll + p;
}

/* the chroma position of absolute luma position p (16.16) under subsampling shift k, clamped to the
 * chroma columns the run [s0, s0 + len) covers */
KF_DEVICE long long kf_cpos(long long p, unsigned int k, unsigned int s0, unsigned int len)
{
	long long c = ((p + 32768ll) >> k) - 32768ll;
	long long lo = (long long)(s0 >> k) * 65536ll;
	long long hi = (long long)((s0 + len - 1u) >> k) * 65536ll;
	return c < lo ? lo : (c > hi ? hi : c);
}

KF_DEVICE unsigned int kf_lerp(unsigned int v00, unsigned int v10, unsigned int v01,
			       unsigned int v11, unsigned int fx, unsigned int fy)
{
	unsigned long long wx = 65536ull - fx, wy = 65536ull - fy;
	unsigned long long top = v00 * wx + (unsigned long long)v10 * fx;
	unsigned long long bot = v01 * wx + (unsigned long long)v11 * fx;
	return (unsigned int)((top * wy + bot * fy + 2147483648ull) >> 32);
}

/* One output pixel (x, row) of the destination rectangle. */
KF_DEVICE unsigned int kf_yuv_pixel(const kf_yuv_args *a, unsigned int x, unsigned int row)
{
	unsigned int y, c0, c1;
	if (!a->filter) {
		unsigned long long sx = a->sx0 + (unsigned long long)x * a->sw / a->dw;
		unsigned long long sy = a->sy0 + (unsigned long long)row * a->sh / a->dh;
		y = kf_luma(a, sx, sy);
		kf_chroma(a, sx >> a->sxl, sy >> a->syl, &c0, &c1);
	} else {
		long long px = kf_pos(x, a->dw, a->sx0, a->sw), py = kf_pos(row, a->dh, a->sy0, a->sh);
		unsigned long long x0 = (unsigned long long)(px >> 16), y0 = (unsigned long long)(py >> 16);
		unsigned long long xe = a->sx0 + a->sw - 1u, ye = a->sy0 + a->sh - 1u;
		unsigned long long x1 = x0 < xe ? x0 + 1ull : xe, y1 = y0 < ye ? y0 + 1ull : ye;
		unsigned int fx = (unsigned int)(px & 65535ll), fy = (unsigned int)(py & 65535ll);
		y = kf_lerp(kf_luma(a, x0, y0), kf_luma(a, x1, y0), kf_luma(a, x0, y1),
			    kf_luma(a, x1, y1), fx, fy);
		long long cpx = kf_cpos(px, a->sxl, a->sx0, a->sw);
		long long cpy = kf_cpos(py, a->syl, a->sy0, a->sh);
		unsigned long long cx0 = (unsigned long long)(cpx >> 16);
		unsigned long long cy0 = (unsigned long long)(cpy >> 16);
		unsigned long long cxe = (a->sx0 + a->sw - 1u) >> a->sxl;
		unsigned long long cye = (a->sy0 + a->sh - 1u) >> a->syl;
		unsigned long long cx1 = cx0 < cxe ? cx0 + 1ull : cxe;
		unsigned long long cy1 = cy0 < cye ? cy0 + 1ull : cye;
		unsigned int cfx = (unsigned int)(cpx & 65535ll), cfy = (unsigned int)(cpy & 65535ll);
		unsigned int a0, a1, b0, b1, d0, d1, e0, e1;
		kf_chroma(a, cx0, cy0, &a0, &a1);
		kf_chroma(a, cx1, cy0, &b0, &b1);
		kf_chroma(a, cx0, cy1, &d0, &d1);
		kf_chroma(a, cx1, cy1, &e0, &e1);
		c0 = kf_lerp(a0, b0, d0, e0, cfx, cfy);
		c1 = kf_lerp(a1, b1, d1, e1, cfx, cfy);
	}
	return kf_convert(a->m, y, a->vu ? c1 : c0, a->vu ? c0 : c1);
}

/* One CTA per destination row, threads striding the row. */
KF_DEVICE void kf_yuv_row(const kf_yuv_args *a, unsigned int *dst, unsigned int row,
			  unsigned int thread)
{
	for (unsigned int x = thread; x < a->dw; x += KF_YUV_THREADS) {
		unsigned int dx = a->ox + x, dy = a->oy + row;
		if (dx >= a->fw || dy >= a->fh)
			continue;
		dst[(unsigned long long)dy * a->fw + dx] = kf_yuv_pixel(a, x, row);
	}
}

/* The same window into the SDR colour pipeline's FP32 staging frame (4 floats a pixel, alpha 1): the
 * converted 8-bit value / 255, stored opaque in the window's place in the back-to-front order. The
 * window's colour pipeline after the FMT matrix (ILUT, CSC) is not applied (the console's view only). */
KF_DEVICE void kf_yuv_row_f(const kf_yuv_args *a, float *dst, unsigned int row, unsigned int thread)
{
	for (unsigned int x = thread; x < a->dw; x += KF_YUV_THREADS) {
		unsigned int dx = a->ox + x, dy = a->oy + row;
		if (dx >= a->fw || dy >= a->fh)
			continue;
		unsigned int p = kf_yuv_pixel(a, x, row);
		unsigned long long d = ((unsigned long long)dy * a->fw + dx) * 4ull;
		dst[d + 0] = (float)((p >> 16) & 255u) / 255.0f;
		dst[d + 1] = (float)((p >> 8) & 255u) / 255.0f;
		dst[d + 2] = (float)(p & 255u) / 255.0f;
		dst[d + 3] = 1.0f;
	}
}

/* the launch parameters, in this order, for both entries (and the host harness) */
#define KF_YUV_PARAMS                                                                              \
	const unsigned char *ys, const unsigned char *cs, unsigned int bl, unsigned int yp,          \
		unsigned int cp, unsigned int bh, unsigned int sx0, unsigned int sy0,               \
		unsigned int sw, unsigned int sh, unsigned int ox, unsigned int oy,                 \
		unsigned int dw, unsigned int dh, unsigned int fw, unsigned int fh,                 \
		unsigned int sxl, unsigned int syl, unsigned int vu, unsigned int layout,           \
		unsigned int filter, int m00, int m01, int m02, int m03, int m10, int m11, int m12, \
		int m13, int m20, int m21, int m22, int m23

#define KF_YUV_ARGS                                                                                \
	{ ys, cs, bl, yp, cp, bh, sx0, sy0, sw, sh, ox, oy, dw, dh, fw, fh, sxl, syl, vu, layout,  \
	  filter, { m00, m01, m02, m03, m10, m11, m12, m13, m20, m21, m22, m23 } }

#ifndef KF_HOST
extern "C" __attribute__((global)) void kf_color_yuv(float *dst, KF_YUV_PARAMS)
{
	kf_yuv_args a = KF_YUV_ARGS;
	kf_yuv_row_f(&a, dst, __nvvm_read_ptx_sreg_ctaid_x(), __nvvm_read_ptx_sreg_tid_x());
}

extern "C" __attribute__((global)) void kf_compose_yuv(unsigned int *dst, KF_YUV_PARAMS)
{
	kf_yuv_args a = KF_YUV_ARGS;
	kf_yuv_row(&a, dst, __nvvm_read_ptx_sreg_ctaid_x(), __nvvm_read_ptx_sreg_tid_x());
}
#endif
