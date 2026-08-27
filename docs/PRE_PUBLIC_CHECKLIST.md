# Pre-public checklist — review before any external/public launch

Items deliberately parked as "good enough to proceed, but revisit before we go
public." Not blockers for current development; each has a recorded root cause so
it can be picked up as a focused effort.

---

## NVENC encode throughput — task #101 (RE-BASELINED 2026-06-02: NOT a blocker)

**Status:** re-measured cleanly; the "~7×" was a measurement artifact. NVENC
encode is correct and usable (#99). Apples-to-apples (RTX 3060, drv 580,
h264_nvenc p4, pre-generated raw input, identical cmd both sides):

| input path             | host | guest | ratio |
|------------------------|-----:|------:|------:|
| 720p,  CPU raw         |  932 |   895 | 0.96× (PARITY) |
| 1080p, CPU raw         |  428 |    63 | 6.8×  |
| 1080p, CUDA `hwupload` |  302 |   121 | 2.5×  |

The old "55 vs 373 / 7×" were short-clip cumulative-average artifacts + CPU-bound
`testsrc` input. NOT eventfd, NOT fence coherence, NOT WC cacheability — all three
theories disproven (see `docs/design/async_event_delivery.md`).

**Real cause:** at 1080p with CPU raw input the guest main ffmpeg thread is
memcpy-bound (8/8 gdb samples in `av_image_copy` into NVENC's CPU input surface,
which nvkvm has migrated onto the GPA window) — per-frame multi-MB CPU writes
*through the window* are slower in-guest than native (likely EPT TLB pressure).
Steady-state ioctls ≈ 0; mapping the buffer WB gave zero change (already WB via #94).

**Why not a blocker:** 720p at parity; 1080p still 63–121 fps = real-time for
1080p30/1080p60 streaming. The real use case (GPU-resident framebuffer → NVENC)
never touches the slow CPU input surface — the 6.8× is a CPU-raw-input artifact.

**Deferred perf fix (same family as #94):** huge-page-back the GPA window memslot
(THP / `MADV_HUGEPAGE` in QEMU) — a 3 MB per-frame write hits 768 4 KB EPT entries
vs ~2 with 2 MB pages; benefits ALL large guest window accesses (HtoD/DtoH/NVENC).

Details: task #101; memory `nvenc_101_root_cause_wc_input.md`.
