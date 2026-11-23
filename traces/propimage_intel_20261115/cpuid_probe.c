#include <stdio.h>
#include <string.h>
#include <intrin.h>
static void leaf(unsigned l, unsigned s) { int r[4]; __cpuidex(r, l, s); printf("leaf %08x.%u eax=%08x ebx=%08x ecx=%08x edx=%08x\n", l, s, r[0], r[1], r[2], r[3]); }
int main(void) {
  int r[4]; char v[13] = {0}; __cpuid(r, 0); memcpy(v, &r[1], 4); memcpy(v + 4, &r[3], 4); memcpy(v + 8, &r[2], 4);
  printf("vendor=%s maxleaf=%u\n", v, r[0]);
  leaf(1, 0); leaf(7, 0); leaf(0xd, 1); leaf(0x80000000, 0); leaf(0x80000001, 0); leaf(0x80000008, 0);
  char b[49] = {0}; for (int i = 0; i < 3; i++) { __cpuid(r, 0x80000002 + i); memcpy(b + 16 * i, r, 16); } printf("brand=%s\n", b);
  __cpuid(r, 0x40000000); char h[13] = {0}; memcpy(h, &r[1], 4); memcpy(h + 4, &r[2], 4); memcpy(h + 8, &r[3], 4); printf("hv_vendor=%s maxhv=%08x\n", h, r[0]);
  leaf(0x40000001, 0); leaf(0x40000003, 0); leaf(0x40000004, 0);
  unsigned eax = r[0]; (void)eax; unsigned fam, model, step; __cpuid(r, 1); step = r[0] & 15; model = (r[0] >> 4) & 15; fam = (r[0] >> 8) & 15;
  if (fam == 15) fam += (r[0] >> 20) & 255; if (fam >= 6) model += ((r[0] >> 16) & 15) << 4;
  printf("decoded family=%u model=%u stepping=%u\n", fam, model, step);
  return 0; }
