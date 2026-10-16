STATUS: MEASURED, 2026-10-16. All new FPS results are clean c66ac9de, production configuration. Final adapter 4b2c2878 has a failed Linux gate and no Heaven timing here.

# Heaven comparison

Host driver 595.91.07; actual Windows guest driver 580.88; same owner Win11 backing image
through an owned overlay. Eight vCPUs, 16 GiB guest RAM, 8192 MiB framebuffer, 8 GiB guest
BAR1; Heaven 4.0, 1920×1080, windowed, RELEASE,QUALITY_ULTRA, in-app F9 benchmark.
No PresentMon. No CPU stress overlaps these measurements. Guest nvidia-smi sampler and
host dmon run at approximately 1 Hz. Actual KF3 environment is in P2_VALIDATION.md;
full launch lines and /proc environments remain in the host evidence directories.

| API | Inherited 0a746117 median FPS | WINDOWED median (min–max) FPS | FULL median (min–max) FPS | Inherited VFIO median FPS | WINDOWED / native | FULL / native | FULL / WINDOWED change |
|---|---:|---:|---:|---:|---:|---:|---:|
| direct3d11 | 97.65 | 113.364 (111.491–114.807) | 113.638 (110.297–113.890) | 330.05 | 34.3% | 34.4% | +0.24% |
| opengl | 222.50 | 225.976 (224.685–231.191) | 228.393 (226.861–228.427) | 341.90 | 66.1% | 66.8% | +1.07% |

FULL failed the preregistered >10% speedup criterion for both APIs. Mapping the full store
simplifies access but does not remove the read latency. These medians remain below 90%
of the inherited VFIO references. Reciprocal median FPS gives D3D11 8.82 ms/frame WINDOWED,
8.80 FULL, versus native 3.03; OpenGL 4.43/4.38 versus native 2.93. These are derived average
frame durations, not an attribution of CPU, GPU or semaphore time.

The D3D11 WINDOWED median is 16.1% above the inherited 0a746117 median; that is a comparison
between different source revisions. No fresh 0a746117 control was completed after staging
Heaven, so this session does not attribute that difference to the shared-view change.
The inherited baseline/native numbers come from Claude's final campaign report at 9b2d7fe1b;
its KF3_FULLMAP_EXP was never run. This session used its own production FULL property,
with no experiment-branch merge or dirty binary.

Raw logs: `/data/codex-perf/ev/heaven/`, with one directory per mode/API/repetition.
`results.jsonl` contains all twelve numbers. Mode directories contain session logs, actual
executable paths, host/guest driver queries, process environments, private launch lines and
full QEMU shutdown logs. Each repetition includes the benchmark's HLOG output, host dmon,
per-thread CPU census and its QEMU log slice. Raw files remain on the host. The per-revision
executables were removed afterward with owned builds, as instructed; build logs retain their
source paths and revisions.
