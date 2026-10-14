# OpenGL gears investigation

**STATUS: ANSWERED, 2026-10-11.** Starts at integration `c1aa06b0`.

Run 643 uses the validated product binary `02afce90` with the GSP observer and corrected
sign-in harness. Hypothesis: boot-time static information or a previously missed control
reply diverges before the gears crash. Falsifier: all relevant model-owned static fields
and pre-crash replies agree with the working VFIO reference. A difference is a candidate,
not proof of causation; it needs a source-backed correction and a controlled run.

Pair replies by ordered function/client/object/command matches, not RPC sequence alone:
the existing reference uses sequence zero repeatedly. Observer polling timestamps do not
measure precise guest request-to-reply latency. Raw captures and closed-driver diagnosis
stay private; only behavior-level findings and open-source derivation enter this tree.

## Static graphics capabilities hypothesis

Measured run 643 reproduces gears' access violation with the GSP observer enabled; CUDA probes
pass, adapter Code 0, zero TDR. Its fn65 reply has zero `grCapsBits`; the VFIO reference has
nonzero bits. Other differences include deliberate virtual FB sizes/root addresses and unused
physical-board/RTD3/displayless fields. This narrows a candidate, not causation.

Open sources (`ogkm-580.65.06`: `gsp_static_config.h:80`, `ctrl2080gr.h:1455`,
`kernel_graphics.c:850-851,929,1172-1186,2568-2571`) establish the shared GR caps layout.
The implementation reuses the existing populated host `GR_GET_CAPS_V2` query; no new per-die
row or captured constant. Windows' use of the fn65 member is a hypothesis, not demonstrated
by those Linux source call sites. A synthetic regression failed with zero output; it now passes,
including served-chain tests at 535/580/590 and absent-member tests at 595/610. Independent
review found no correctness/provenance blocker and requested the absent-member coverage, applied.

Controlled run 644 changes only this static field in the product. Falsifier: gears still exits
with access violation despite the populated field. Oracle: gears reports the NVIDIA GL renderer,
remains alive while frames advance, and exits successfully after its workload; CUDA and adapter
status must remain good. The binary source revision and actual result follow after the run.

## Run 644 verdict and narrower probe

`18bfb458` populated fn65 GR caps with the same queried bytes as VFIO; gears still exited
-1073741819. The stated falsifier fired. CUDA cup2/3/8 passed, adapter Code 0, zero TDR.
The GPU-free ABI/RM suite passed; this change is not an OpenGL fix and is not integrated.

An instrumented build of the open-source gears probe selected pixel format 9, set that format,
and returned a non-null WGL context. It crashed during `wglMakeCurrent`, before GL renderer
queries. The runner was paused while these additional probes ran, then resumed for teardown.
Its recorded hold is not an independent endurance test. Probe stage output is in `logs/644/`.

Next diagnostic: run the same probe on VFIO and compare driver-call outcomes around that
boundary. Hypothesis: a CPU-side query or client action not represented in the gears GSP stream
fails differently on kayfabe. Falsifier: corresponding calls have matching successful results.
Raw diagnostic buffers stay private. No product capability is changed based on an inferred
physical field; in particular, remaining fn65 memory geometry is not assumed to share Windows'
semantics solely because the body size agrees with open Linux headers.

## CPU-side bus census hypothesis (runs 645 and VFIO diagnostic boot 2)

The same private syscall tracing probe reproduced the MakeCurrent crash on kayfabe;
VFIO completed 43418 frames in 20 seconds, NVIDIA renderer, correct pixel, GL error 0,
RESULT OK. Both sessions stopped cleanly, host nvidia restored, group 11 DMA-FQ, Xid count
unchanged at 35. The first tracer attempt recursed and was discarded; these observations
use the corrected tracer. Raw diagnostic buffers remain private.

A CPU-side bus-information census was entirely zero on kayfabe. The public GSP trace
shows its first missing forwarded index: BUS_GET_INFO_V2 index 0x18 (ASLM status), refused
NV_ERR_NOT_SUPPORTED. VFIO completes the subsequent 0x19..0x2b link-status/error census.
Open `ogkm-580` `kern_bus_ctrl.c:333` returns on the first failed forwarded query.
Separately, the Windows observation is that the enclosing caller discarded the partial
census. This is a candidate explanation for the GL failure, not proof.

Candidate derives the *virtual* PCIe link: present without ASLM, valid disabled ASPM,
zero link-switch/line/CRC/NAK/L0s/AER error counters and clear variants. Definitions:
`ogkm-580` `ctrl2080bus.h:308-327,479-502`. No physical link status is copied. Unknown indices still
refuse the whole request. The served-boundary regression failed first with status 86;
all eight bus-info tests pass after the change.

Controlled run 646 hypothesis: completing this census restores the initialization data
that gears needs. Falsifier: the census completes but gears still crashes. Oracle remains
NVIDIA renderer, correct pixel, advancing frames, RESULT OK; a non-null WGL context alone
is insufficient. Source revision and result will be recorded after the run.

Independent review found no blocker in the bus candidate; the fixed-size answers apply
uniformly across families and preserve request validation. GPU-free `kf-abi` + `kf-rm`
tests: 1300 passed across 69 test/doc-test binaries, zero failures (candidate `2c917f3e`).

Run 646 (`2c917f3e`) completed the formerly refused bus queries, but sign-in verification
failed; no interactive gears probe ran. The harness exited 1 and restored DMA-FQ/nvidia,
no QEMU remaining. This is not a gears verdict. Run 647 repeats the same binary with a
longer harness boot readiness wait and verified sign-in retries; falsifier/oracle unchanged.

## Run 647: gears passes

Exact product `2c917f3e` changes only the bus-census handling relative to run 645's
`18bfb458`. After a verified sign-in retry, the same traced probe passes: 43605 frames
in 20 seconds, NVIDIA 580.88 renderer, pixel 64/128/191/255, GL error 0, RESULT OK.
The CPU-side bus table is now populated. The stated falsifier did not fire.

The ordinary app-disk `D:\tools\kf_glgears.exe` then passed independently of the private
probe hooks: **95688 frames in 45.0 seconds, GL error 0, RESULT OK, exit 0**, NVIDIA renderer
and correct pixel. CUDA cup2/cup3/cup8 each exited 0 and passed their result checks;
adapter Code 0, zero TDR. Full output, the exact interactive task and a display screenshot
are in `logs/647/`. Empty ExitCode from the earlier Start-Process helper is not treated as
an exit-zero claim; the ordinary task invokes the executable directly and records LASTEXITCODE.

This demonstrates that completing the census is sufficient in this comparison. It does not
isolate which previously discarded bus field the ICD requires. The GR-caps change alone
was insufficient (run 644). Linux regressions on the exact passing product are pending.
Only our stopped runs' 645/646 disposable overlays were removed after evidence collection
to preserve the host's 100 GB free-space floor; logs and private diagnostic captures remain.

## Exact-product Linux regression

`2c917f3e` passed the ABI/RM GPU-free suite locally (1300 tests), host display/RM/QEMU
GPU-free suite, hardware gates 9/9 plus gate 10, rebuilt QEMU/client/fast guest, fast suite
30/30, T-space log checks 30/30, and broker/input proof. The outer GPU lock serialized all
stages. `linux/` carries the run markers, gates, arm verdicts, T-space checks, broker proof
and binary hash; full per-arm logs remain on the trusted host under
`codex-gl-regression-20261011-2c917f3e/`. Regression exited 0 at 13:15:15 CEST.

Run 648 is a fresh Windows boot of the same product with RPC tracing, QEMU trace and GSP
observer disabled. It uses the ordinary open-source probe with no private hooks and
records its native LASTEXITCODE. Result/teardown follow when complete. This checks the
passing outcome without measurement flags; it is not a performance comparison.

## Final normal-profile verdict

Run 648 (`2c917f3e`), fresh baseline and measurement flags off: **93693 frames in 45.0s,
NVIDIA renderer, pixel 64/128/191/255, GL error 0, RESULT OK, native exit 0**. Adapter Code 0,
zero TDR; runner exited 0. The recorded command confirms RPC/QEMU tracing and the GSP
observer are off, display is on. Host restored to nvidia and DMA-FQ, no QEMU or own stop
file, 104 GiB free. `logs/648/` contains the launch profile, probe result and teardown.

The candidate branch is pushed; integration/master remain unchanged. Their merge gates
still apply. The broader OpenGL application matrix is a separate outstanding lane.

Run 648's single Running→Suspending transition belongs to the terminal driver unload
(fn47, newLevel 64) during shutdown and goes directly to Halted; it is not a TDR/recovery
cycle. `logs/648/driver-phases.txt` records all driver phase transitions for that distinction.
