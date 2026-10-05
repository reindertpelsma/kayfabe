# Windows J: checked memory registration advances initialization

**STATUS: RESEARCH, 2026-10-05.** Windows still reports Code 43; no GPU workload
or successful GPU-channel birth is established.

Product: `fdc991c9d3fd51dab005beda2487365661a3db25`, branch
`codex/guest-memory-list-probe-20261005`. Runner: `152afe7b`, fresh overlay, RTX4070
AD104, native Linux/open595.91.07 host, Windows580.88 using guest ABI580.65.06.
The independent pool, real timer, private Translated-space, software-runlist
allocation and memory-registration experiments are enabled. Virtual GOP is off;
auxiliary VGA is present. This is Kayfabe, not VFIO.

Both function-4 memory registrations return NV_OK. Windows proceeds to four
software-runlist metadata objects, its timer, a VA space and display resources:
one core DMA channel, eight window channels, eight immediate windows and four
cursors. There are 363 traced RPCs and 365 serviced messages, compared with 213
and 215 in I. These counts include cleanup and are not a measure of useful GPU
work. Display writes, PUTs and methods remain zero; GPU-channel births remain
zero. The 21 display channels are emulated display objects, not GPU passthrough
channels.

The later repeated `0x50700117` requests set one-shot flags for the next RmFree;
they are cleanup, not polling. The two scheduling controls `0x20801111` are still
refused during the later cleanup sequence, now with non-null registered memory
handles and zero counts. No scheduling-success rule is inferred. Timeout
configuration `0x20801110` also remains refused.

The checked registration contract and its nine explicit source-supported guest
rows are documented on the [product branch](https://github.com/reindertpelsma/kayfabe/blob/fdc991c9/tools/memory-list-probe/README.md).
It adds no host RM call or GPU mapping. All compatibility and lifetime restrictions
remain in force; older/unknown producer contracts fail closed. Local validation
was 140 Rust tests plus the production C RAM-section predicate fixture. This boot
is one hardware/driver tuple, not all-driver/all-family or hostile-isolation
validation. The preceding pool and software-runlist probes remain explicitly
incomplete diagnostic behavior.

`status-first.stdout` records Code43 and NVIDIA-SMI exit9. Its registry listing
also reports denied access to the protected Properties key; the NVIDIA driver
settings were read. `failure-status.stdout` preserves the PnP details; that script
exited1 after an empty event-query path, with no stderr output. The new WATCHDOG
live dump was analysed with the already pinned and Microsoft-signed KD bundle:
exit0, semantic analysis complete, live dump0x1b0/StartDevice/c000009a. This status
is not proof of actual RAM exhaustion. The NVIDIA journal has new assertions;
the [causal analysis](../../../tools/windows-debug-capture/evidence/2026-10-05-startdevice-j.md)
identifies a TMO-buffer precondition. The subsequent [L experiment](../probe-l/README.md)
passes that helper and reaches a later display constructor failure.
Raw dump, tagged bytes, full decoded journal and debugger text stay private on
the controller, not solely on the borrowed PC.

Windows shut down through authenticated SSH; QEMU exited0. The Linux host GPU
remained available through595.91.07. No physical GPU rebinding or firmware change
was involved. Files here contain the command, build log, RPC/log evidence, guest
status, clean shutdown and run context.
