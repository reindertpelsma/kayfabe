# Mode-2 cuCtxCreate diagnostic LD_PRELOAD shims

Boot-free (host) + guest ioctl/mmap tracers used to diagnose the cuCtxCreate
crash. Build: `cc -shared -fPIC -O0 -o X.so X.c -ldl`. Run:
`LD_PRELOAD=./X.so <cuda-app>`.

- **seqshim.c** — logs the RM ioctl sequence (alloc class / control cmd / map / vidheap).
- **fillshim.c** — tracks MAP_SHARED mmaps and reports which ioctl first makes
  each region non-zero ([FILL]) — pins the "who fills this buffer" question.
- **guestshim.c** — combined: alloc class + map + mmap size, for guest-vs-host diff.

## Key 2026-06-05 finding (host vs guest cuCtxCreate diff)
Host (real GPU) cuCtxCreate: ONE shared mmap (2MiB GPFIFO), stays zero, PASSES.
Guest (Mode-2): maps 2MiB GPFIFO + **64MiB + 4KiB** extra (the GR context buffers)
in SYSMEM (CPU-mapped) -> libcuda reads zero -> rbp=0 SIGSEGV. The 64MiB GR ctx is
VIDMEM on the host (GPU-only, never CPU-read). Root divergence = guest allocates GR
ctx in sysmem (FBMEM_PREFERRED fell back to sysmem); host uses vidmem. CPU-RM regkey
RMInstLoc/2/3 forcing VID did NOT change it (GSP/PMA-side decision).
