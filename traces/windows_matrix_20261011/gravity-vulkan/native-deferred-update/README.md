# Native authored deferred channel-update experiment

**STATUS: RESEARCH, 2026-10-11.** Native experiment FAIL (host Xid32),
no production call site. The earlier hardware-pending paragraph below is history.

**Correction, 2026-10-13:** the later fully prepared relocation at source
`1a666d783` PASSes both as UID 65534 / CapEff zero and as UID 0 on the same
RTX 4070 / 595.91.07. Matched NOP controls do not move; trigger cases execute a
replacement-only GPU marker and advance replacement USERD GET, with zero host
Xids. [Raw evidence](../prepared-native-migration/README.md). This supersedes
the historical "not currently observed" and native-path-unavailable conclusions
below. The same-config faults remain measured; their inner cause is unknown.
Cursor continuity, relay integration and other drivers remain unproved.

At20:17UTC the lifetime discriminator also FAILS natively: NOP clean, update
and update-hold each cause Xid32 despite rc0 and a GPU tail. The hold fault
arrives300us after HOLD_BEGIN, about2s before resources are released, ruling
out early cleanup for this native case. Root diagnostic reads return bounded
records but no decoded inner status. See
[`rc-journal`](../rc-journal/README.md). Registration privilege/permissions
remain an investigation lead, not a measured failure status or Windows fix.

## Correction 2026-10-12 — parameter preparation and missing inner status

The coordinator's validity question is justified. Re-reading the retained
harness and encoder: native memory64KiB was allocated and mapped; GPFIFO at
VA+0x1000 (32entries) and client USERD in the same live object at offset0x2000
are exactly the values that successfully birthed the channel. Public V2 CPU-RM
registers hUserdMemory with GSP before INTERNAL; V2 registration returned OK.
Owned client/channel/object ancestry and encoded offsets/counts are checked.
The update-hold case retained all resources for2s after a real GPU tail, and
faulted during that hold. Thus the test did not intentionally submit dangling
memory or an unmapped replacement ring.

It nevertheless did **not** prepare a new ring/USERD relocation or prove the
same-config inner request valid with a successful privileged control. Native
GSP acceptance constraints/inner status remain unmeasured. Both uid0 and65534
faulted in the later reference; UID alone does not establish RM registration
privilege. A tail cannot prove this nonstalling inner method succeeded. The
bounded public RC journal supplied opaque nested bytes, no named inner status;
there is no decoded NV_ERR_INSUFFICIENT_PERMISSIONS result to cite.

Conclusion: this exact same-config experiment faults; it does not prove every
valid unprivileged migration impossible or identify permission as the cause.
No product path depends on that inference. An alternative requires a known-valid
prepared migration/positive reference, a readback of actual relocation/progress,
and a per-host-driver capability probe. Successful execution would require an
explicit security/authority review before use; it is not currently observed.

RC-scope source was read on origin/claude/rc-scope-20261012. GSP recovery policy
is absent; M1-M3 remain required before any cross-VM availability claim. The
current shadow experiment is exclusive single VM and grants no CPU API action.

## Proposed stream-preserving implementation, not yet validated

The owner's suggested direction is to register an authored host channel-info
update while retaining the deferred trigger in the GPU stream. Unlike our
mapping/invalidate gate, this action can in principle be executed by native RM
at the stream trigger. GPGA containment alone is insufficient: validate the
target channel/space ownership, complete FIFO and USERD spans, alignment and
entry count, and retain the backing/mappings through the hardware transition.
Substitute our host client/channel/memory handles; do not forward guest control
bytes. An unchanged numeric API handle also requires collision/lifetime checks
in the host deferred object's namespace. Kayfabe's channel metadata and USERD
tracking must follow the same transition, rather than switching at registration.

Current Windows USERD is specifically a relayed guest-RAM slot, not an adopted
GPGA USERD: the real host channel uses a private kayfabe-owned4KiB USERD object
in no GPU VA space (`kf-qemu/src/chan.rs`, Relay and passthrough birth). A guest
USERD move must therefore switch the validated relay endpoint at the proper
stream transition while preserving that private host object. A FIFO resize
also changes the relay's entry-count bound and ring metadata. Merely registering
the guest's new USERD address would not preserve this existing contract.

Public source preserves registration privilege for the inner deferred control;
unprivileged V2 registration does not itself establish permission to execute
the privileged FIFO update. Exact595.91.07 generated subdevice flags remain0x4.
The measured native failure blocks promotion of this proposed path, but does
not identify its inner failure status or prove every unprivileged equivalent
impossible. The next isolated hold test separates asynchronous action failure
from early resource cleanup; it does not test actual USERD migration yet.

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
