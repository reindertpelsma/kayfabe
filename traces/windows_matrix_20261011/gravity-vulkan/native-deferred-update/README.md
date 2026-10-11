# Native authored deferred channel-update experiment

**STATUS: RESEARCH, 2026-10-11.** Hardware pending; no production call site.

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
