# Native authored deferred channel-update experiment

**STATUS: RESEARCH, 2026-10-11.** Native experiment FAIL (host Xid32),
no production call site. The earlier hardware-pending paragraph below is history.

## Measured result, 2026-10-11 18:56:51 UTC

Both native processes returned0, registered V2, and observed an actual GPU tail
(NOP1055us, update1064us); cleanup calls returned success. Those facts do NOT
establish that the asynchronous deferred software action succeeded. The kernel
records Xid32 at18:56:51.221583UTC, PID287987, namekf-deferred-upd, channel4,
intr00800000. Per-mode PID attribution was not recorded, so the faulty mode is
unmeasured. The hypothesis's no-Xid requirement failed; no Windows fix follows.

The original kernel capture wrongly reported0 because --until truncated the
current timestamp to18:56:51.000000UTC, excluding the fault's fractional second.
Original and corrected journals are both preserved under result/. The capture
helper now uses microseconds and accepts an explicit endpoint, with the bounded
known-fault window verified to contain1Xid. The clpeak run's supplementary
capture also crossed into this later native test; corrected-journal/ limits it
to its own stop time and contains only its context-switch Xid109.

## Isolated repeat, 2026-10-11 19:02:55–59 UTC

The SAME binary (SHA256 1c94782825cec20674817541546ff915aa74b94019772963754ab9383c1fa25a)
ran both modes with actual process PIDs and separate precise windows, including
two seconds of bench-only delayed-fault observation after process exit:

| Mode | PID | Process rc | Host Xids | Verdict |
| --- | ---: | ---: | ---: | --- |
| NOP | 296840 | 0 | 0 | PASS |
| Update | 296888 | 0 | 1 (Xid32) | FAIL |

Raw logs and precise-window verdicts are in isolated-result/. Both cases ran;
overall rc1. This attributes the fault to the update case, falsifying this exact
unprivileged authored path. It does not identify the inner RM rejection status.
The GPU-written tail still cannot prove asynchronous software-action success.
The observation interval never releases product state or invents completion.


Run1288 GravityMark Vulkan times out611s with a device-lost/submit/present
dialog, three guest driver events, one GSPcycle and host Xid32/channel0x4e.
Faulting twin has an allocated5080 object with no registered host actions.
That is a correlation, not identification of the faulting method. The queued
bounded nbody diagnostic now includes Vulkan in a separate fresh guest.

Public source provides a possible authored equivalent previously missed by
the direct-control analysis: NV5080_CTRL_CMD_DEFERRED_API_V2 is NON_PRIVILEGED
(ogkm580.65.06 g_deferred_api_nvoc.c flags0x10008). Its CPU-RM implementation
registers the caller's USERD memory with GSP, then calls INTERNAL with the
same V2 structure (deferred_api.c:327-394). At the software trigger, the
recorded registration privilege is used for the inner control (lines550-654).
Whether an unprivileged caller can execute this FIFO update is UNMEASURED.
This does not justify forwarding the guest's registration or raising host
channel privilege. Direct FIFO_UPDATE_CHANNEL_INFO remains privileged.

New bench-only HostRm method builds V2 from exact compiler-derived host fields,
always this session's client and owned channel/object/USERD memory. It rejects
foreign ancestry, invalid API handles, malformed ring spans/alignment and
unmeasured layouts. The runtime product has no caller. GPU-free host tests pass;
build/test logs saved alongside. No new per-die constant or captured layout.

The native harness registers the SAME owned configuration in both invocations.
It queries the actual software classID, then submits SET_OBJECT plus either
5080 NO_OPERATION or the real deferred trigger, followed by an actual GPU WFI
semaphore tail. CPU initializes the fence and never completes it. USER channel
assertion required. Both modes run as uid65534, no capabilities, exclusively
under the shared GPU lock. Original deadline10s, outer30s supervisor; a timeout
fails and frees the owned channel, never opens a product wait.

Hypothesis: the unprivileged V2 path can execute the authored update. Falsifier:
registration refusal, missing GPU tail, failed cleanup or host Xid. Success
would prove only this same-config native path, NOT USERD migration, safe guest
operand translation, cause of run1288, or a repaired Windows app.

Queued inside existing native-event job71386 while flock150604 had no child;
the original video case script is unchanged and retained separately. The batch
records each experiment's exit status independently, then releases the GPU.
No source build occurs on the host during an app timing run. This native test
precedes corrected-stream and nbody/Vulkan snapshot jobs, then the already
queued frozen3252 regression/manual session. No input is sent to the owner VM.
