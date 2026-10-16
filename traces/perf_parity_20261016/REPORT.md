STATUS: STOPPED, 2026-10-16. Linux regression gate failed at 04:14 CEST; do not merge this draft.

Stopped under section 6: clean final P2 source `4b2c28784c8066b4c0299300427e014becf16d5b`
returned **29/30** in the Linux fast suite. `--defer-liveness` failed with raw client rc=1;
the same arm and suite previously passed on `c66ac9de` (30/30). No failing arm was retried.
P3 hardware work and later items were not started. The final one-hour soak was canceled
at 04:16:34, before its required completion. The PR remains draft and blocked.

Branch: `codex/perf-parity-20261016`, based on `0a74611734dc91c78c86ff39ed9f06d866072ac4`.
Draft PR: https://github.com/reindertpelsma/kayfabe/pull/17 against `claude/perf-fix-20261015`.
No merge, experiment-branch merge, owner-VM stop, VFIO detach or Vast instance was performed.

**P1 — partial verification of 0a746117.** kf-qemu tests: 290 passed. kf-chan/kf-mem tests
passed on the host. Gates 9/9, gate 10 PASS, 12/12 USER births, fast suite 30/30.
Linux broker reached KMS/Cinnamon/LINEAR dmabuf; separately verified interactive baseline
used the NVIDIA renderer and shut down cleanly. The old cursor/window hook was not graded
under GNOME Wayland. The first broker attempt omitted the guest-driver override and refused
at realize; correcting it to the installed 580.159.04 allowed the lane to run. This was not
a guest regression. Independent Windows Heaven repetitions on the baseline were not completed
within P1; its numbers below remain inherited. See P1_VERDICT.md for the exact limits.

**P2 — implementation and measured comparison, blocked by final Linux gate.** USER rings
and Translated/probe readers share one store allocation view indexed by physical offset.
Default WINDOWED has a global budget of 256 windows, atomic reader pins, bounded clock
reclamation, owner accounting and named refusal/fallback counters. FULL is an explicit,
default-off realize property; hardware initialization and Heaven runs succeeded with it.
Each mapping access is bounded to 256 bytes and excludes the firmware carve. The C/Rust
configuration seam is ABI 31. `user-view-keep` is deprecated rather than reinterpreted.

Review found that the first Translated adapter still armed views under its channel guard.
Commit 4b2c2878 corrects this with deferred requests and maintenance after guards end;
retirement and temporary fallback release also leave caller guards before native calls.
Four tests cover guarded misses, refusal, concurrent retirement and fallback cleanup.
Earlier c66ac9de retains that requirement gap and is not offered as a mergeable alternative.

| Windows Heaven 4.0, Ultra, 1920×1080 | Inherited 0a746117 FPS | c66ac9de WINDOWED FPS | c66ac9de FULL FPS | Inherited VFIO FPS | FULL / native |
|---|---:|---:|---:|---:|---:|
| D3D11, median of 3 | 97.65 | 113.364 | 113.638 | 330.05 | 34.4% |
| OpenGL, median of 3 | 222.50 | 225.976 | 228.393 | 341.90 | 66.8% |

FULL versus WINDOWED: +0.24% D3D11, +1.07% OpenGL. The >10% speedup falsifier was not met.
Neither result is within 10% of native. The +16.1% D3D11 difference from the inherited
baseline needs a fresh baseline control before causal attribution. No same-revision
performance contradiction was established. Full spreads and provenance: HEAVEN.md.

All new Heaven numbers use clean c66ac9de, default Cargo features, actual Windows driver
580.88, host 595.91.07, eight vCPUs, 16 GiB RAM, 8192 MiB framebuffer and 8 GiB BAR1.
The copied production harness's actual KF3 variables are listed in P2_VALIDATION.md and
archived from /proc; PERF_FLAGS is empty, with store-view-full explicitly off/on.
Native/baseline numbers are inherited from Claude's final campaign report at 9b2d7fe1b.
Its dirty KF3_FULLMAP_EXP binary was not used; that experiment had never been run.

| P2 validation | c66ac9de | Final adapter 4b2c2878 |
|---|---|---|
| Touched crates | 772 passed, 0 failed | 776 passed after documented dev environment repair |
| Actual-slot Loom model | PASS | PASS |
| Hardware gates | 9/9; gate 10; 12 USER births | 9/9; gate 10; 12 USER births |
| Linux fast suite | 30/30 | **29/30, defer-liveness failed** |
| Broker / interactive Linux | Rendering smoke passed; cursor hook ungraded | Not reached after fast-suite failure |
| Memfd stress, 8/16/32 threads × 600 s | All passed; bytes verified, arms=releases, fds/maps at baseline | Protocol unchanged; adapter tests added |
| Final 32-thread, 3600 s soak | — | Interrupted by stop rule; no pass claimed |
| Read scaling, 1..32 threads | Reported in P2_VALIDATION.md | Same slot protocol |

Stress seeds: 539365406 / 539365414 / 539365430. Host affinity 0–7; final interrupted dev
soak affinity 0–3. Separate-span RAM reads reached 15.861 million/s at 32 oversubscribed
threads; same-span contention returned Retry and reached 5.890 million/s. These are
protocol/RAM numbers, not BAR1 bandwidth. No shared lock remains in the view protocol.

The dev-box combined test command initially failed three existing descriptor tests because
/dev/null was a regular tmpfs file. Its bytes were preserved privately, then char 1:3 mode
0666 was restored atomically. The affected raw crate and previously skipped QEMU crate
passed afterward; successful kf-chan results were retained. The aggregate is 776 passes.
No code was changed to suppress those assertions. Transient fastguest /proc environments
were not separately archived; their launchers cleared KF3 variables and raw launch evidence
is retained. Windows and interactive Linux process environments were captured.

**The failed gate — measured facts and unresolved cause.** The failing guest logs show
NVIDIA driver 595.91.07. Host/QEMU logs first refuse the 2 GiB guest-RAM OS descriptor as
Other(30), then report T-space not built and channel birth refused. The guest driver fails
initialization; the client fails opening nvidia<gpu> with EIO. Public NVIDIA status headers
identify decimal 30 (0x1e) as invalid address. This identifies the failure point, not its
cause or a causal link to the deferred adapter. A later read-only host check had about
25.5 GiB MemAvailable; there is no measured basis here for claiming memory exhaustion.

Raw evidence:

- `/data/codex-perf/fastwork/codex_shared_4b2c2878_suite.out`
- `/data/codex-perf/fastwork/fast_codex_shared_4b2c2878_defer-liveness_{qemu,serial,ttyS0}.log`
- `/data/codex-perf/logs/p2-final-{build,hardware,v3-gates}.log`
- `/data/codex-perf/ev/heaven/` (twelve runs, host dmon, thread census, benchmark output, process metadata)
- `/data/codex-perf/logs/p2-{unit-loom,all-touched-tests,stress-8,stress-16,stress-32,read-scaling}.log`
- `/workspace/codex-perf/private/logs/p2-final-{touched-tests,after-devnull-repair}.log`
- `/workspace/codex-perf/private/logs/p2-soak-60min-local.log` (explicitly canceled)

**P3 — preparation only.** The opt-in store-read-bench example compares current copy,
8-byte loads, 16/32-byte vectors, independent loads and requested UC/WC mappings with
fenced MOVNTDQA. Unit checks cover exact widths, unaligned tails, boundaries and refusals.
The optional owned copy-engine control checks GPU-written data after its real fence.
Compilation passed; no hardware read-cost or GPU-freshness result was obtained. No winner
was installed in production and no command payload reads were skipped. P3_CODE_AUDIT.md
explains which framing/binding/trigger words the checker needs and the remaining admission
and private-staging constraints. No GPU ring gather/translator was implemented.

**P4–P6 — not reached after the stop.** No new per-frame attribution, native exit census,
doorbell-to-GPU-start measurement, host patch list, Blender/CUDA/LLM comparison or large-page
implementation/design was completed. Existing OptiX/huge-page/compression findings remain
inherited research, not new validation by this session.

Principal commits: a50bed673 preregistration; f0c35acde slot prototype/model;
392012990 typed Translated read Retry; aafc66355 shared runtime integration;
c66ac9de1 scaling and measured candidate; 4b2c28784 deferred guard correction;
7e5ca732f offline GPU-writer probe and evidence. The branch contains smaller API/test/doc
commits between these points. All code and reports are retained for review; nothing merged.

**Final host state, verified 04:32 CEST.** No QEMU VM or GPU holder remains; lock is free.
RTX 4070 is on NVIDIA 595.91.07, P8, 9 MiB used / 11864 MiB free. Owned overlays, Cargo
build targets, QEMU source/build copies, per-revision binaries and installer media were
deleted as instructed. Raw logs, source checkouts and bundles remain. Disk free is
97,350,246,400 bytes (97.35 GB / 90.67 GiB), 95% used. No service was stopped; no GPU
reattach or Vast teardown was necessary. Five lingering pairs of this session's private
run3191 brokers and its HTTP server were terminated by verified exact command arguments.
Owner/default broker sockets and other processes were left alone. Final state evidence:
`/data/codex-perf/logs/FINAL_HOST_STATE.log`. Rebuild binaries and recreate owned overlays
before resuming. The dev box's owned Cargo target is also removed after preserving logs.

Ranked next work:

1. Resolve the failed guest-RAM import before merging: inspect readiness, host address/range,
   mapping lifetime and driver admission at kf-qemu mem.rs:2715 / T-space prewarm. Compare
   the c66 passing and 4b2 failing logs. These are candidates, not established causes.
2. After a concrete fix, rerun the failing arm and the required complete fast/gates/broker/
   interactive checks, then complete the one-hour final soak. Do not treat a clean retry
   alone as a fix. c66's guard gap also needs the corrected adapter.
3. Re-establish the fresh 0a746117 Heaven control; run the prepared bare-host read probe
   unprivileged, verify effective cache attributes and clocks, then implement/test its
   bounded winner and measure the new per-frame copy cost.
4. Complete the Windows submission budget before broader changes. Keep host patch and
   proper large-page/compression work as reviewable designs; then proceed to Blender/CUDA.
