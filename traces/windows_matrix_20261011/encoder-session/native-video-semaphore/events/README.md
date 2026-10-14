# Native video notification experiment

**STATUS: RESEARCH, 2026-10-11.** Hardware experiment FAIL; bench only.

## Measured result, 2026-10-11 18:56:48–51 UTC

| Case | GPU semaphore / tail | NVENC ready | FIFO ready | Process rc |
| --- | --- | --- | --- | ---: |
| release/events | actual completion | false | false | 0 |
| trap/events | actual completion | true | false | 0 |
| release/fifo-nsi | actual completion | true | false | 1 |
| trap/fifo-nsi | NOT RUN | — | — | — |

All executed cases had quiet200ms listener baselines. The FIFO NSI positive
control failed to wake the FIFO listener, so the runner stopped before case4.
The first pair measures a trap-related NVENC wake on this host, but the failed
control leaves the full notification hypothesis incompletely validated.
Overall experiment rc1. No guest notification forwarding or translated-video
admission follows. Raw evidence: result/probe.log. The original whole-second
kernel capture has the endpoint limitation documented in the deferred-update
experiment; it is not used to establish absence of a delayed fault.

## Original hypothesis and protocol

The existing authored C9B7 release/trap oracle now optionally registers separate
dataless NVENC and FIFO_EVENT_MTHD listeners, using the existing public-source
HostRm event API. Both listeners must have a quiet200ms baseline after clearing
initial readiness. The GPU-written semaphore and WFI tail remain the completion
oracle. Event readiness never supplies completion.

Run release/events, trap/events, release/fifo-nsi, trap/fifo-nsi on NVENC1.
The first pair differs only by the video trap. The second pair adds an authored
FIFO NON_STALL_INTERRUPT as a positive control for the FIFO listener. Each event
is sampled for200ms after the actual GPU tail. A missing FIFO positive-control
wake fails the probe. No NVENC wake is assumed or forged.

Hypothesis: C9B7 TRAP wakes the NVENC non-stall listener without an explicit
FIFO interrupt. Falsifier: quiet baseline and valid FIFO positive control, but
no NVENC wake for trap/events. Record both listeners for every case even if the
hypothesis is false. These GPU-wide listeners cannot identify a channel, and
the experiment requires exclusive GPU work with no QEMU.

This does not implement guest notification forwarding or admit translated video
submission. Original release/trap invocations keep their existing behavior.
