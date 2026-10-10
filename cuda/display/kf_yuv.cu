/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * kf_yuv.cu - the console copy's semi-planar YUV window kernel (a window whose SET_PARAMS.FORMAT is
 * Y8___V8U8_N420 and its N422/N444 U8V8 siblings: Edge's video overlay plane, 2026-10-10).
 *
 * One window is converted (BT.709, limited range, 8.8 fixed point) and written OPAQUE into the
 * compose kernel's XRGB8888 staging frame, scaled from the source rectangle to the destination
 * rectangle by nearest-neighbour sampling. Plane 0 is Y8; plane 1 holds interleaved chroma byte
 * pairs, one per 2^sxl x 2^syl luma pixels. Both planes are pitch-linear or NVIDIA block-linear
 * (the address is kf_disp::scanout::bl_offset, as in kf_scanout.ptx).
 *
 * Bounds are the HOST's job (kf_disp::scanout::plan_yuv, kf_cuda::display::DisplayGpu::compose_yuv):
 * every address formed here lies inside the extents the plan checked against the context DMAs and
 * the store, every write inside the fw x fh frame. The CPU reference is
 * kf_disp::scanout::yuv_reference / yuv_to_xrgb; kf-disp's tests/yuv_kernel.rs runs THIS body on
 * the host (-DKF_HOST) over every (CTA, thread) and compares byte for byte.
 *
 * ONE SOURCE, TWO COMPILERS (cuda/display/make_yuv_ptx.sh for the device). Plain C only below.
 */
#ifdef KF_HOST
#define KF_DEVICE static
#else
#define KF_DEVICE static __attribute__((device))
#endif

#define KF_YUV_THREADS 256u

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

KF_DEVICE unsigned int kf_q(int x)
{
	x = (x + 128) >> 8;
	return (unsigned int)(x < 0 ? 0 : (x > 255 ? 255 : x));
}

KF_DEVICE unsigned int kf_yuv_to_xrgb(int y, int u, int v)
{
	int c = 298 * (y - 16), d = u - 128, e = v - 128;
	return 0xff000000u | (kf_q(c + 459 * e) << 16) | (kf_q(c - 55 * d - 136 * e) << 8) |
	       kf_q(c + 541 * d);
}

/* One output pixel (x, row) of the destination rectangle. */
KF_DEVICE unsigned int kf_yuv_pixel(const unsigned char *ys, const unsigned char *cs,
				    unsigned int bl, unsigned int yp, unsigned int cp,
				    unsigned int bh, unsigned int sx0, unsigned int sy0,
				    unsigned int sw, unsigned int sh, unsigned int dw,
				    unsigned int dh, unsigned int sxl, unsigned int syl,
				    unsigned int vu, unsigned int x, unsigned int row)
{
	unsigned long long sx = sx0 + (unsigned long long)x * sw / dw;
	unsigned long long sy = sy0 + (unsigned long long)row * sh / dh;
	unsigned long long cx = sx >> sxl, cy = sy >> syl;
	int yv = ys[kf_addr(bl, sx, sy, yp, bh)];
	int c0 = cs[kf_addr(bl, cx * 2ull, cy, cp, bh)];
	int c1 = cs[kf_addr(bl, cx * 2ull + 1ull, cy, cp, bh)];
	return kf_yuv_to_xrgb(yv, vu ? c1 : c0, vu ? c0 : c1);
}

/* One CTA per destination row, threads striding the row. */
KF_DEVICE void kf_yuv_row(const unsigned char *ys, const unsigned char *cs, unsigned int *dst,
			  unsigned int bl, unsigned int yp, unsigned int cp, unsigned int bh,
			  unsigned int sx0, unsigned int sy0, unsigned int sw, unsigned int sh,
			  unsigned int ox, unsigned int oy, unsigned int dw, unsigned int dh,
			  unsigned int fw, unsigned int fh, unsigned int sxl, unsigned int syl,
			  unsigned int vu, unsigned int row, unsigned int thread)
{
	for (unsigned int x = thread; x < dw; x += KF_YUV_THREADS) {
		unsigned int dx = ox + x, dy = oy + row;
		if (dx >= fw || dy >= fh)
			continue;
		dst[(unsigned long long)dy * fw + dx] =
			kf_yuv_pixel(ys, cs, bl, yp, cp, bh, sx0, sy0, sw, sh, dw, dh, sxl, syl, vu,
				     x, row);
	}
}

/* The same window into the SDR colour pipeline's FP32 staging frame (4 floats a pixel, alpha 1): the
 * converted 8-bit value / 255, stored opaque in the window's place in the back-to-front order. The
 * window's own colour pipeline is not applied (the console's view only; see kf_disp::scanout::plan_yuv). */
KF_DEVICE void kf_yuv_row_f(const unsigned char *ys, const unsigned char *cs, float *dst,
			    unsigned int bl, unsigned int yp, unsigned int cp, unsigned int bh,
			    unsigned int sx0, unsigned int sy0, unsigned int sw, unsigned int sh,
			    unsigned int ox, unsigned int oy, unsigned int dw, unsigned int dh,
			    unsigned int fw, unsigned int fh, unsigned int sxl, unsigned int syl,
			    unsigned int vu, unsigned int row, unsigned int thread)
{
	for (unsigned int x = thread; x < dw; x += KF_YUV_THREADS) {
		unsigned int dx = ox + x, dy = oy + row;
		if (dx >= fw || dy >= fh)
			continue;
		unsigned int p = kf_yuv_pixel(ys, cs, bl, yp, cp, bh, sx0, sy0, sw, sh, dw, dh, sxl,
					      syl, vu, x, row);
		unsigned long long d = ((unsigned long long)dy * fw + dx) * 4ull;
		dst[d + 0] = (float)((p >> 16) & 255u) / 255.0f;
		dst[d + 1] = (float)((p >> 8) & 255u) / 255.0f;
		dst[d + 2] = (float)(p & 255u) / 255.0f;
		dst[d + 3] = 1.0f;
	}
}

#ifndef KF_HOST
extern "C" __attribute__((global)) void
kf_color_yuv(const unsigned char *ys, const unsigned char *cs, float *dst, unsigned int bl,
	     unsigned int yp, unsigned int cp, unsigned int bh, unsigned int sx0, unsigned int sy0,
	     unsigned int sw, unsigned int sh, unsigned int ox, unsigned int oy, unsigned int dw,
	     unsigned int dh, unsigned int fw, unsigned int fh, unsigned int sxl, unsigned int syl,
	     unsigned int vu)
{
	kf_yuv_row_f(ys, cs, dst, bl, yp, cp, bh, sx0, sy0, sw, sh, ox, oy, dw, dh, fw, fh, sxl, syl,
		     vu, __nvvm_read_ptx_sreg_ctaid_x(), __nvvm_read_ptx_sreg_tid_x());
}

extern "C" __attribute__((global)) void
kf_compose_yuv(const unsigned char *ys, const unsigned char *cs, unsigned int *dst, unsigned int bl,
	       unsigned int yp, unsigned int cp, unsigned int bh, unsigned int sx0, unsigned int sy0,
	       unsigned int sw, unsigned int sh, unsigned int ox, unsigned int oy, unsigned int dw,
	       unsigned int dh, unsigned int fw, unsigned int fh, unsigned int sxl, unsigned int syl,
	       unsigned int vu)
{
	kf_yuv_row(ys, cs, dst, bl, yp, cp, bh, sx0, sy0, sw, sh, ox, oy, dw, dh, fw, fh, sxl, syl,
		   vu, __nvvm_read_ptx_sreg_ctaid_x(), __nvvm_read_ptx_sreg_tid_x());
}
#endif
