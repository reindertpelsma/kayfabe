# Native video notification experiment

**STATUS: RESEARCH, 2026-10-11.** Hardware pending; bench only.

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
