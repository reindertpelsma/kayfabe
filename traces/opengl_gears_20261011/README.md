# OpenGL gears investigation

**STATUS: RESEARCH, 2026-10-11.** Starts at integration `c1aa06b0`.

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
