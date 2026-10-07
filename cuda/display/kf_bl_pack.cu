/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * kf_bl_pack.cu - the GPU-copy broker rung's pack kernel (docs/design/V3_DISPLAY.md sec. 8.11,
 * OWNER_RULINGS.md sec. L).
 *
 * The finished frame (the compose kernel's pitch-linear staging frame, XRGB8888, 4*w bytes a
 * row, the cursor already composed in grab mode) is written into a VRAM object kayfabe allocated
 * itself (a "slot") in NVIDIA block-linear layout, which the compositor imports as a dma-buf with
 * DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D. No byte crosses PCIe; nothing of the guest's is exported.
 *
 * One warp per 512-byte GOB, one thread per 16-byte chunk: thread c of GOB g writes slot bytes
 * [512*g + 16*c, +16), the pixels at kf_disp::vramslot::bl_chunk_origin(g, c) - the exact inverse
 * of kf_disp::scanout::bl_offset, so every byte of the extent is written exactly once, pixels
 * inside the w x h frame copied and the padding zeroed. The in-GOB byte order is SETUP DATA
 * (THE_CONSTRAINTS.md sec. 21): the five bit positions of y[0], y[1], x[4], y[2], x[5] are kernel
 * parameters (GA106: 4, 5, 6, 7, 8), so a family that differs is a data change.
 *
 * Bounds are the HOST's job (kf_cuda::display::DisplayGpu::bl_pack): src holds 4*w*h bytes, dst
 * holds gobs*512 bytes, the layout is a permutation of 4..8. This kernel reads src only at
 * y < h, x < w, and writes dst only below gobs*512.
 *
 * ONE SOURCE, TWO COMPILERS:
 *  - the device: clang's NVPTX back-end, no CUDA SDK (`cuda/display/make_pack_ptx.sh`), committed
 *    as cuda/display/kf_bl_pack.ptx;
 *  - the host (-DKF_HOST, plain C): kf-disp's test `tests/bl_pack_kernel.rs` runs this same body
 *    over every (block, thread) and compares the slot byte for byte with
 *    kf_disp::vramslot::pack_reference.
 * Plain C only below (no C++), so the host build is the same text the device build compiles.
 */

#ifdef KF_HOST
#define KF_DEVICE static
typedef struct {
	unsigned int x, y, z, w;
} kf_u4;
#else
#define KF_DEVICE static __attribute__((device))
typedef unsigned int kf_u4 __attribute__((ext_vector_type(4)));
#endif

/* The threads of one CTA: eight GOBs. */
#define KF_PACK_THREADS 256u

/* One thread's chunk. `block` and `thread` are the CTA and thread index. */
KF_DEVICE void kf_bl_pack_chunk(const unsigned int *src, kf_u4 *dst, unsigned int w,
				unsigned int h, unsigned int gpr, unsigned int bh,
				unsigned long long gobs, unsigned int by0, unsigned int by1,
				unsigned int bx4, unsigned int by2, unsigned int bx5,
				unsigned int block, unsigned int thread)
{
	unsigned long long g = (unsigned long long)block * (KF_PACK_THREADS / 32u) + (thread >> 5);
	unsigned int c = thread & 31u;
	if (g >= gobs)
		return;
	/* the GOB's place: in_block GOBs down a block, gob_x across, block_y blocks down */
	unsigned long long in_block = g & ((1ull << bh) - 1ull);
	unsigned long long t = g >> bh;
	unsigned long long gob_x = t % gpr;
	unsigned long long block_y = t / gpr;
	unsigned long long gob_y = (block_y << bh) | in_block;
	/* the chunk's bits, read where the layout puts them */
	unsigned int o = c << 4;
	unsigned long long x_bytes = (gob_x << 6) | (((o >> bx5) & 1u) << 5) | (((o >> bx4) & 1u) << 4);
	unsigned long long y = (gob_y << 3) | (((o >> by2) & 1u) << 2) | (((o >> by1) & 1u) << 1) |
			       ((o >> by0) & 1u);
	unsigned long long px = x_bytes >> 2;
	unsigned int p0 = 0, p1 = 0, p2 = 0, p3 = 0;
	if (y < h) {
		const unsigned int *row = src + y * (unsigned long long)w;
		if (px + 0 < w)
			p0 = row[px + 0];
		if (px + 1 < w)
			p1 = row[px + 1];
		if (px + 2 < w)
			p2 = row[px + 2];
		if (px + 3 < w)
			p3 = row[px + 3];
	}
	kf_u4 v;
	v.x = p0;
	v.y = p1;
	v.z = p2;
	v.w = p3;
	dst[g * 32ull + c] = v;
}

#ifndef KF_HOST
/* The entry: grid = ceil(gobs / 8) CTAs of KF_PACK_THREADS threads. */
extern "C" __attribute__((global)) void kf_bl_pack(const unsigned int *src, kf_u4 *dst,
						    unsigned int w, unsigned int h,
						    unsigned int gpr, unsigned int bh,
						    unsigned long long gobs, unsigned int by0,
						    unsigned int by1, unsigned int bx4,
						    unsigned int by2, unsigned int bx5)
{
	kf_bl_pack_chunk(src, dst, w, h, gpr, bh, gobs, by0, by1, bx4, by2, bx5,
			 __nvvm_read_ptx_sreg_ctaid_x(), __nvvm_read_ptx_sreg_tid_x());
}
#endif
