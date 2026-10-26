# Phase 3 frame distributions

MEASURED 2026-10-26, run `lx2-phase3-20261026`, variants `02-N` through `13-N`:
each row summarizes six runs of 1000 frames. Median/p90/p99 cells are medians
of the six per-run statistics; maximum is over all 6000 frames. Spike counts
sum frames exceeding twice their own run's median. Exact per-run statistics,
ranges and separate sampled-pass results are in [phase3-variants.json](phase3-variants.json).
Depth-only omits the sampled pass. These are offscreen timings, not presentation gaps.
All 60000 variant completion gaps are below 33/50/100 ms; this short campaign
does not test long-session hitch behavior.

| Variant/arm/interval | Median ms | p90 ms | p99 ms | Max ms | >2x own-run median /6000 |
|---|---:|---:|---:|---:|---:|
| base/N/render_gpu | 0.778240 | 0.779264 | 0.783360 | 0.908288 | 0 |
| base/N/completion_gaps | 0.812288 | 0.816879 | 1.144156 | 2.461888 | 10 |
| base/K/render_gpu | 1.367040 | 1.370624 | 1.374725 | 3.062784 | 3 |
| base/K/completion_gaps | 1.421760 | 1.430716 | 3.074774 | 4.953291 | 102 |
| colour/N/render_gpu | 0.777216 | 0.777216 | 0.782336 | 0.800768 | 0 |
| colour/N/completion_gaps | 0.810375 | 0.814853 | 1.127623 | 2.082281 | 7 |
| colour/K/render_gpu | 0.776192 | 0.777216 | 0.782341 | 2.285568 | 6 |
| colour/K/completion_gaps | 0.828043 | 0.835116 | 2.505344 | 6.864960 | 153 |
| depth/N/render_gpu | 0.192512 | 0.192512 | 0.194048 | 0.214016 | 0 |
| depth/N/completion_gaps | 0.205356 | 0.207735 | 0.396436 | 2.697476 | 48 |
| depth/K/render_gpu | 0.584192 | 0.585216 | 0.588805 | 2.180096 | 6 |
| depth/K/completion_gaps | 0.607253 | 0.612109 | 2.074753 | 4.186540 | 108 |
| small/N/render_gpu | 0.107520 | 0.107520 | 0.109066 | 0.236544 | 6 |
| small/N/completion_gaps | 0.129859 | 0.134186 | 0.430884 | 1.913663 | 117 |
| small/K/render_gpu | 0.185344 | 0.186368 | 0.187392 | 2.886656 | 6 |
| small/K/completion_gaps | 0.227003 | 0.233233 | 1.708991 | 4.470520 | 156 |
| large/N/render_gpu | 3.060736 | 3.062784 | 3.065856 | 3.086336 | 0 |
| large/N/completion_gaps | 3.133554 | 3.137803 | 3.529446 | 5.609869 | 0 |
| large/K/render_gpu | 5.544960 | 5.557248 | 5.595653 | 7.293952 | 0 |
| large/K/completion_gaps | 5.724920 | 5.763007 | 7.492924 | 14.598930 | 2 |
