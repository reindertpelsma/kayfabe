STATUS: IN PROGRESS, 2026-10-16 02:54 CEST. Final soak and Heaven comparison pending.

# Shared store validation

Production code/build: clean `c66ac9de1863e802dc3b8e275e100e23db800562` on
`codex/perf-parity-20261016`. Default Cargo features, WINDOWED, global budget 256,
no behavioral `KF3_*` flags. Host driver 595.91.07. Linux guest driver 580.159.04.
Source after this commit adds documentation and an explicitly enabled offline probe only.
Hardware executable: `/data/codex-perf/kf3-bins/c66ac9de/qemu-system-x86_64`.

| Check | Result | Raw evidence on host |
|---|---|---|
| kf-qemu + kf-chan + kf-linux-raw | 772 passed, 0 failed, 2 long tests ignored here | `/data/codex-perf/logs/p2-all-touched-tests.log` |
| Loom, actual slot state and reclamation | Passed, two readers and one evictor | `/data/codex-perf/logs/p2-unit-loom.log` |
| gates | 9/9; gate 10 PASS; 12/12 USER channel births | `/data/codex-perf/logs/p2-v3-gates.log` |
| fast suite | 30/30 | `/data/codex-perf/fastwork/codex_shared_c66ac9de_suite.out` |
| Linux broker | KMS/Cinnamon/LINEAR dmabuf passed; no captured kernel fault | `/data/codex-perf/logs/p2-linux-broker.log` |
| Linux interactive | Verified c66 executable; NVIDIA renderer; glxgears 60.326/59.997 fps; no captured Xid/Flip/WARN/BUG | `/data/codex-perf/logs/p2-interactive-guest.log` |
| FULL explicit realize | Full permitted store mapping succeeded, QMP quit, exit 0 | `/data/codex-perf/logs/p2-full-control.log` |
| 8 threads, 600 s, seed 539365406 | 3,538,961 verified reads; 6,467,172 arms = releases; fds 4→4, owned maps 0→0 | `/data/codex-perf/logs/p2-stress-8.log` |
| 16 threads, 600 s, seed 539365414 | 2,579,478 verified reads; 6,489,937 arms = releases; fds 4→4, owned maps 0→0 | `/data/codex-perf/logs/p2-stress-16.log` |
| 32 threads, 600 s, seed 539365430 | 211,524 verified reads; 6,392,319 arms = releases; fds 4→4, owned maps 0→0 | `/data/codex-perf/logs/p2-stress-32.log` |
| Final 32-thread, 3600 s soak | Running on dev box, 02:58:09 CEST start | `/workspace/codex-perf/private/logs/p2-soak-60min-local.log` |
| Read scaling, 1/2/4/8/16/32 threads | Passed; memfd data, not BAR1 | `/data/codex-perf/logs/p2-read-scaling.log` |

CPU stress uses affinity 0–7 and nice 10; thread counts above eight are oversubscribed.
Every returned byte is checked. No failed run has been retried. Loom and adversarial tests
cover arming failures, partial mapping ownership, cleanup refusal, a stalled pinned reader,
thread exit and concurrent hostile-owner eviction. Source tests reject Mutex/RwLock and wait
primitives in the mapping protocol. No shared lock remains to histogram; an atomic reader
pin remains until the bounded copy ends.

The broker harness's legacy cursor/window hook cannot locate its window under the current
GNOME Wayland desktop; that hook is NOT GRADED. The separately verified interactive launch
covers the NVIDIA renderer and rendering smoke, not a substitute claim of input-hook grading.
The first interactive attempt selected 0a746117 because its wrapper cleared KF3_REV. That
run is retained solely as baseline control in P1_VERDICT; it is excluded from this table.

`KF3_REV=c66ac9de` is launch metadata, not a behavioral switch. Actual process environment
and command line were saved privately as `p2-interactive-kf3-env.txt` and
`p2-interactive-cmdline.txt`; they are not committed because the launch includes owner identity.
The FULL control ran without a guest disk and with no KF3 variables. It is an initialization
check, not a Heaven performance number. Linux smoke FPS is display capped and is not a
throughput comparison with VFIO.

Claude's final report says its KF3_FULLMAP_EXP was scripted but never run. Its dirty
experiment binary is not used. FULL/WINDOWED Heaven comparisons on this production revision
remain required; no speedup claim is made.

The host soak was deliberately canceled after its start, without a failing assertion,
to move CPU-only work away from the performance host. Its incomplete log is not a pass.
The full dev-box soak uses clean source `00ba4613e4da091096a2774a926e43c934e5f76b`,
whose store-view protocol, native integration, USER pump and Translated readers are byte-for-byte
identical to c66ac9de (git diff checked). Default features; 32 threads on affinity 0–3.
The later source commits add the gated offline probe and evidence only.

Read scaling on c66ac9de, affinity 0–7, three seconds per case, 64-byte reads:

| Threads | Same-span M reads/s | Same-span p50/p99 ns | Separate-span M reads/s | Separate-span p50/p99 ns |
|---|---:|---|---:|---|
| 1 | 2.198 | 100/130 | 2.186 | 100/130 |
| 2 | 4.056 | 130/180 | 4.209 | 110/160 |
| 4 | 7.696 | 150/220 | 8.120 | 120/170 |
| 8 | 5.904 | 540/1190 | 13.775 | 140/540 |
| 16 | 5.817 | 650/1260 | 14.560 | 120/540 |
| 32 | 5.890 | 650/1240 | 15.861 | 120/390 |

Same-span contention returns Retry rather than waiting; separate-span runs reported zero
retries. Timing and byte verification are included in throughput, so this is protocol/RAM
scaling rather than a BAR1 bandwidth number. The same-span result saturates near six million
verified reads/s; separate spans reached 15.861 million/s at 32 oversubscribed threads.
