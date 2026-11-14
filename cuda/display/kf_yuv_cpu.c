/* kf_yuv_cpu.c — the YUV kernel's body on a CPU, with the plane ending at a guard page (2026-11-13).
 *
 * STATUS: LIVE. Security review 3 F-6: with an odd POINT_IN.x + SIZE_IN.width, the packed 4:2:2
 * layouts read the straddling macropixel's second chroma byte 1-2 bytes past the extent the host
 * checks ((rows - 1) * pitch + 2 * (sx0 + sw), crates/kf-cuda/src/display.rs). Here the plane is
 * exactly that many bytes long and ends where a PROT_NONE page begins, so an over-read is a SIGSEGV of
 * the child that runs the case, not an unnoticed 2 bytes. Every pixel of every case is computed.
 * Also: for an even sx0 + sw the output is the one the same pixels give inside a wider window
 * (the fix changes nothing there), and on an odd one only the straddling macropixel's pixels differ.
 *
 *   cc -O1 -Wall -Wextra -DKF_HOST -o kf_yuv_cpu kf_yuv_cpu.c && ./kf_yuv_cpu     (scripts/ci: job ptx)
 */
#define KF_HOST 1
#include "kf_yuv.cu"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

static int g_fail = 0;

/* A buffer of exactly `n` bytes whose last byte is followed by an unmapped page. */
static unsigned char *guarded(size_t n, void **base, size_t *len)
{
	size_t pg = (size_t)sysconf(_SC_PAGESIZE);
	size_t body = (n + pg - 1) / pg * pg;
	*len = body + pg;
	unsigned char *m = mmap(NULL, *len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if (m == MAP_FAILED)
		abort();
	if (mprotect(m + body, pg, PROT_NONE))
		abort();
	*base = m;
	unsigned char *p = m + body - n;
	for (size_t i = 0; i < n; i++)
		p[i] = (unsigned char)(i * 37u + 11u);
	return p;
}

/* Render the whole destination of a packed window (1:1, filter as given) into `out`. */
static void render(unsigned int layout, unsigned int filter, unsigned int sx0, unsigned int sy0,
		   unsigned int sw, unsigned int sh, unsigned int pitch, unsigned int *out,
		   const unsigned char *plane)
{
	kf_yuv_args a;
	memset(&a, 0, sizeof(a));
	a.ys = a.cs = plane;
	a.yp = a.cp = pitch;
	a.sx0 = sx0; a.sy0 = sy0; a.sw = sw; a.sh = sh;
	a.dw = sw; a.dh = sh; a.fw = sw; a.fh = sh;
	a.sxl = 1u; a.syl = 0u; a.layout = layout; a.filter = filter;
	int m[12] = {91881, 76309, 0, -14260, -46802, 76309, -22554, 8676, 0, 76309, 116130, -18071};
	memcpy(a.m, m, sizeof(m));
	for (unsigned int row = 0; row < sh; row++)
		for (unsigned int t = 0; t < KF_YUV_THREADS; t++)
			kf_yuv_row(&a, out, row, t);
}

/* One case in a child: the plane is exactly the host-checked extent. Returns 0 if no fault. */
static int no_overread(unsigned int layout, unsigned int filter, unsigned int sx0, unsigned int sy0,
		       unsigned int sw, unsigned int sh, unsigned int pitch)
{
	pid_t pid = fork();
	if (pid == 0) {
		size_t extent = (size_t)(sy0 + sh - 1u) * pitch + 2u * (size_t)(sx0 + sw);
		void *base;
		size_t len;
		unsigned char *p = guarded(extent, &base, &len);
		unsigned int *out = calloc((size_t)sw * sh, 4);
		render(layout, filter, sx0, sy0, sw, sh, pitch, out, p);
		_exit(0);
	}
	int st = 0;
	waitpid(pid, &st, 0);
	return WIFEXITED(st) && WEXITSTATUS(st) == 0 ? 0 : 1;
}

#define EXPECT(c, ...) do { if (!(c)) { g_fail++; printf("  FAIL %s:%d: ", __FILE__, __LINE__); \
	printf(__VA_ARGS__); printf("\n"); } } while (0)

int main(void)
{
	/* the review's model example, and odd ends around it, both packed orders, both filters */
	const unsigned int odd[][5] = {
		{37, 11, 32, 4, 202}, {0, 0, 1, 1, 64}, {1, 0, 2, 3, 64}, {3, 5, 6, 2, 96}, {0, 0, 127, 2, 256},
	};
	for (unsigned int layout = 2u; layout <= 3u; layout++)
		for (unsigned int filter = 0u; filter <= 1u; filter++)
			for (unsigned int k = 0; k < sizeof(odd) / sizeof(odd[0]); k++) {
				const unsigned int *c = odd[k];
				EXPECT(no_overread(layout, filter, c[0], c[1], c[2], c[3], c[4]) == 0,
				       "layout %u filter %u sx0=%u sw=%u read past the checked extent", layout,
				       filter, c[0], c[2]);
			}
	/* even ends: still inside (control) */
	EXPECT(no_overread(2u, 1u, 36u, 11u, 32u, 4u, 202u) == 0, "even end read past the extent");

	/* output: even sx0 + sw is unchanged against a wider window of the same pixels; odd changes only
	 * the straddling macropixel's pixels (the last one, and nothing left of its pair) */
	{
		const unsigned int pitch = 256u, sh = 3u;
		unsigned char *plane = malloc(pitch * 8u);
		for (unsigned int i = 0; i < pitch * 8u; i++)
			plane[i] = (unsigned char)(i * 53u + 7u);
		unsigned int a[64 * 3], b[64 * 3];
		for (unsigned int layout = 2u; layout <= 3u; layout++) {
			render(layout, 0u, 4u, 1u, 20u, sh, pitch, a, plane);   /* x_end 24, even */
			render(layout, 0u, 4u, 1u, 21u, sh, pitch, b, plane);   /* x_end 25, odd  */
			int same_even = 1, same_left = 1;
			for (unsigned int r = 0; r < sh; r++)
				for (unsigned int x = 0; x < 20u; x++)
					if (a[r * 20u + x] != b[r * 21u + x]) same_left = 0;
			EXPECT(same_left, "layout %u: an odd end changed pixels left of the straddling macropixel", layout);
			render(layout, 0u, 4u, 1u, 22u, sh, pitch, b, plane);   /* x_end 26, even, wider */
			for (unsigned int r = 0; r < sh; r++)
				for (unsigned int x = 0; x < 20u; x++)
					if (a[r * 20u + x] != b[r * 22u + x]) same_even = 0;
			EXPECT(same_even, "layout %u: an even end's pixels differ from a wider window's", layout);
		}
		free(plane);
	}
	printf("==== kf_yuv_cpu: %s ====\n", g_fail ? "FAIL" : "ok");
	return g_fail ? 1 : 0;
}
