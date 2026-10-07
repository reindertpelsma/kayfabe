# Mode-2 M1/M2 — results: stock driver fully bootstraps the (fake) GSP

Status: **M1/M2 DONE** (2026-06-03). The unmodified stock open NVIDIA driver
(580.159.04) now drives the *entire* GPU bring-up against our emulated device
and stalls only at the point where a **live GSP** would respond — the M3
keystone. Companion to [[mode2_m0_m1_progress]], [[mode2_plan]].

## Fake-the-boot register answers that work (verified on hardware)

All in `src/qemu/nvkvm_gpu_emul.c`, trace-driven (answer where rm_init_adapter
stalls in the BAR0 log, re-run):

| stage | reg (BAR0 off) | answer | spike check |
|---|---|---|---|
| chip id | PMC_BOOT_0 0x0 / _42 0xa00 | 0x176000a1 / 0x176a1000 | identity |
| GFW boot | GFW_BOOT_PLM 0x118128 | 0xFFFFFFFF (PLM lowered) | #1 |
| GFW boot | GFW_BOOT 0x118234 | 0x000000FF (COMPLETED) | #1 |
| VBIOS | PROM window 0x300000.. | real GA106 VBIOS (dumped) | — |
| falcon halt | GSP CPUCTL 0x110100 | 0x10 (HALTED) | #3/#4 |
| RISC-V | GSP HWCFG2 0x1100f4 | 0x400 (RISCV ENABLE) | #2 |

Result, in order, each unblocked the next: chip detect → GFW_BOOT poll (2050x)
→ GSP firmware load (needed the blob + 580 alignment) → VBIOS parse ("VBIOS
version 94.06.2F.40.F7") → 75k-access GSP bootstrap → falcon-halt → RISC-V
enable → **full `kgspBootstrap_TU102`**: executes Booter Load, reads fuses
(0x82xxxx = NV_FUSE), programs FALCON_OS, writes the GSP **command queue head**
(0x110c00 = NV_PGSP_QUEUE_HEAD(0)), sends init RPCs, and enters
`kgspWaitForRmInitDone` → `_kgspRpcRecvPoll`.

## Where it stalls now (the M3 entry point)

`kgspWaitForRmInitDone` → `rpcRecvPoll(... NV_VGPU_MSG_EVENT_GSP_INIT_DONE ...)`
polls the **GSP→CPU status message queue in guest sysmem** for the GSP to post
`GSP_INIT_DONE`, with a periodic `kgspHealthCheck` / heartbeat-mailbox read.
Observed BAR0 symptom: an unbounded spin reading **0xbb0080 / 0xbb0084** (+
re-reading GSP HWCFG2 as a liveness check) — a register pair in the RPC/heartbeat
wait loop (0xbb0000 is not in the 575/580 swref; computed at runtime). The real
GSP never runs in fake-the-boot, so the queue write-pointer never advances and
no GSP_INIT_DONE ever appears → the driver waits forever (kernel workqueue spins;
"console_callback hogged CPU").

This is exactly the planned keystone: **faking registers cannot make the GSP
boot — M3 must emulate the GSP-RM message protocol** and synthesize
GSP_INIT_DONE.

## M3 plan (next)

1. **Find the queue GPAs.** The driver hands GSP the LibOS boot-args / message
   queue addresses via `NV_PGSP_FALCON_MAILBOX0/1` writes (and the radix3/WPR
   meta). QEMU records these (it already sees the MMIO writes) and resolves
   them to guest RAM (QEMU maps every GPA — [[mode2_perf_dma_multigpu]]).
2. **Parse the command queue.** Read `GSP_MSG_QUEUE_ELEMENT` / `rpc_message_header_v`
   from the CPU→GSP queue in sysmem; decode the `NV_VGPU_MSG_*` the driver sent
   (kgspSendInitRpcs: SET_REGISTRY, etc.). Reference: `message_queue_cpu.c`,
   `GspStatusQueueInit`. **Rust logic core** ([[mode2_language_rust]]).
3. **Post GSP_INIT_DONE.** Write a valid `rpc_init_done_v17_00` (rpc_result=NV_OK)
   element into the GSP→CPU status queue, advance the write pointer, raise the
   emulated MSI-X ([[mode2_interrupt_delivery]]) so `_kgspRpcRecvPoll` wakes.
4. **Heartbeat.** Answer the GSP-RM heartbeat mailbox so the health check passes.

`RmInitAdapter` returns success once GSP_INIT_DONE is consumed → the stock
driver believes it has a live GPU. That is the proof-of-concept gate.

## Notes

- Stack aligned to 580.159.04 (host driver ver). Full 580 open source checked
  out at host `/root/open-gpu-kernel-modules` (580.159.04) for M3 protocol
  reading — but the in-guest build uses the DKMS tree `/usr/src/nvidia-580.159.04`
  whose RM core is a precompiled blob (so the RM core is NOT instrumentable from
  that tree; use the full open source build if printk instrumentation is needed).
- Runtime stays unprivileged (VBIOS = one-time provisioning asset).

## KEYSTONE PASSED (2026-06-03): GSP_INIT_DONE consumed, driver in live RPC

After the full fake-the-boot chain + msgq handshake (status-queue tx header +
GSP_INIT_DONE post, commit eacea2d), the stock 580 driver's failure moved from
"kgspWaitForRmInitDone / rpcRecvPoll(GSP_INIT_DONE) timeout" to
"_issueRpcAndWait: rpcRecvPoll timedout for fn 1 (SET_GUEST_SYSTEM_INFO)".
=> GSP_INIT_DONE was accepted, kgspBootstrap returned NV_OK, RmInitAdapter
advanced into kgspInitRm and the driver is now issuing real GSP-RM RPCs against
the emulated device. **Fake-the-boot is proven end to end (M0-M3 done).**

Next (M4 RPC shim): respond to each _issueRpcAndWait — read the CPU->GSP command
(cmd queue at sharedMemPA+cmdQueueOffset), post a response element (same fn,
next seqNum, rpc_result NV_OK + expected body) to the status queue, bump
writePtr. SET_GUEST_SYSTEM_INFO (fn 1) first. See
[[mode2_keystone_gsp_init_done]].

## M5 progress 2026-06-03 (commit eb502e9): RPC seq 5 -> 20

Multi-element GSP cmd-consumption bug FIXED (advance cmd_readptr by elemCount, not
1) — the prior 0x3a at GET_CONSTRUCTED_FALCON_INFO was actually corrupted status
queue from mis-reading continuation elements of the 3-element control 0x20800a41.
Driver now runs deep into RmInitNvDevice and stalls at kfifoGetHostDeviceInfoTable
(NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE 0x20801112) returning NV_ERR_NO_MEMORY
on an empty echoed table.

NEXT (capture tool, non-disruptive — host RTX 3060 is idle): build a host
userspace RM client (reuse src/abi/nvgpu.h + tests/integration/test_ioctl_fwd.c
alloc helpers) that opens nvidiactl+nvidia0, allocs NV01_ROOT_CLIENT(0x41) ->
NV01_DEVICE_0(0x80, NV0080_ALLOC_PARAMETERS) -> NV20_SUBDEVICE_0(0x2080,
NV2080_ALLOC_PARAMETERS), then calls 0x20801112 (paginated by baseIndex,
MAX_ENTRIES=32) and dumps the GA106 engine table. Bake the bytes into the
emulator fn-76 handler keyed by cmd 0x20801112. Watch for the device-attach
sequence (REGISTER_FD / NV_ESC_NUMA / attach) that real RM clients need before
subdevice alloc succeeds. See memory mode2_keystone_gsp_init_done #8.
