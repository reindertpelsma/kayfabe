# Mode-2 M3 — GSP-RPC emulation spec (the keystone)

Goal: make `RmInitAdapter` succeed by emulating just enough of the GSP-RM
message protocol to post **GSP_INIT_DONE**, so the stock driver believes it has
a live GPU. Entry state in [[mode2_m2_results.md]]. Structures below are from
the open 580.159.04 source (host `/root/open-gpu-kernel-modules` @ 580.159.04).

## How the driver and GSP share memory

`message_queue_cpu.c` allocates one **shared sysmem region** (`pSharedMemDesc`):

```
shared region (guest RAM, GPA = libos region 'pa'):
  [ command queue ]  CPU->GSP, size commandQueueSize (default 0x40000 = 256KB)
  [ status  queue ]  GSP->CPU, size statusQueueSize   (= pStatusQueue = base + cmdSize)
```

Each queue is an `msgq` SPSC ring whose backing store begins with headers
(`msgq/msgq_priv.h`):

```c
#define MSGQ_VERSION 0
typedef struct {            // 32 bytes, written by the TX side
    NvU32 version;          // = 0
    NvU32 size;             // backing-store bytes, page aligned
    NvU32 msgSize;          // entry size, power-of-2, >=16 (GSP_MSG_QUEUE_ELEMENT_SIZE_MIN)
    NvU32 msgCount;         // number of entries
    NvU32 writePtr;         // msg id of next slot (TX increments)
    NvU32 flags;            // 1 => "i want to swap RX"
    NvU32 rxHdrOff;         // offset of msgqRxHeader from backing-store start
    NvU32 entryOff;         // offset of entries from backing-store start
} msgqTxHeader;
typedef struct { NvU32 readPtr; } msgqRxHeader;  // written by the RX side
```

- **Command queue**: CPU is TX. The driver's `msgqTxCreate` writes the cmd-queue
  txHeader and entries; GSP is RX (reads readPtr/writes... no — GSP writes the
  cmd-queue rxHeader.readPtr to ack consumption).
- **Status queue**: GSP is TX. **We (fake GSP) must write the status-queue
  txHeader** (msgqInit/msgqTxCreate-equivalent) so the driver's `msgqRxLink`
  (in `GspStatusQueueInit`) stops spinning, then write message elements and bump
  writePtr.

`msgqRxLink` spinning for the GSP to init its TX header is the **observed stall**
(BAR0 0xbb0080 liveness poll alongside). Note `GspStatusQueueInit` uses a near
NV_U32_MAX timeout if `IS_EMULATION` — so it waits ~forever.

## How we learn the GPA

`kgspProgramLibosBootArgsAddr_TU102` writes the **LibOS init-args descriptor
GPA** to mailboxes:
```
GPU_REG_WR32(NV_PGSP_FALCON_MAILBOX0 0x110040, lo32(addr));
GPU_REG_WR32(NV_PGSP_FALCON_MAILBOX1 0x110044, hi32(addr));
```
The descriptor is an array (max 4096 entries) of
(`libos_init_args.h`):
```c
typedef struct {
    NvU64 id8;   // id tag (ascii-ish), identifies the region
    NvU64 pa;    // physical address (GPA)
    NvU64 size;  // bytes
    NvU8  kind;  // NONE / CONTIGUOUS / RADIX3
    NvU8  loc;   // NONE / SYSMEM / FB
} LibosMemoryRegionInitArgument;   // (padded)
```
QEMU captures the two mailbox writes → boot-args GPA → `pci_dma_read` the array
→ the message-queue shared region is the SYSMEM/CONTIGUOUS entry (match by id8;
confirm by size == commandQueueSize+statusQueueSize). `pa` = command-queue base;
status queue = `pa + commandQueueSize`.

## The message element

`message_queue_priv.h`:
```c
typedef struct GSP_MSG_QUEUE_ELEMENT {
    NvU8  authTagBuffer[16];   // CC only — zero (CC is OFF)
    NvU8  aadBuffer[16];       // CC only — zero
    NvU32 checkSum;            // 32-bit XOR/sum over the element must be 0
    NvU32 seqNum;              // sequence number (matches CPU expectation)
    NvU32 elemCount;           // # of msgSize elements this message spans
    rpc_message_header_v rpc;  // 8-aligned; { header_version, signature,
                               //   length, function, rpc_result, rpc_result_private,
                               //   sequence, cpuRmGfid, ... , data[] }
} GSP_MSG_QUEUE_ELEMENT;
```
GSP_INIT_DONE = an element whose `rpc.function = NV_VGPU_MSG_EVENT_GSP_INIT_DONE`,
`rpc.rpc_result = NV_OK`, body = `rpc_init_done_v17_00` (the negotiated message
version). `_kgspRpcRecvPoll` special-cases GSP_INIT_DONE (kernel_gsp.c:2324-2329).

## Implementation steps (Rust core + thin C shell)

1. **[C shell] Capture + read.** Trap MAILBOX0/1 writes → boot-args GPA;
   `pci_dma_read` the libos region array; locate the queue region; read the
   command-queue txHeader + the init-RPC element(s) the driver posted. LOG it
   all. (M3-step-1, verifiable now — proves the GPA path + gives ground truth.)
2. **[Rust] Init the status-queue TX header** at status-queue base
   (version=0, size, msgSize=GSP_MSG_QUEUE_ELEMENT_SIZE_MIN, msgCount, writePtr=0,
   rxHdrOff, entryOff) so `msgqRxLink` succeeds.
3. **[Rust] Decode the command queue.** Parse each GSP_MSG_QUEUE_ELEMENT's
   rpc_message_header → the NV_VGPU_MSG_* the driver sent (kgspSendInitRpcs:
   SET_REGISTRY etc.). Ack via the cmd-queue rxHeader.readPtr.
4. **[Rust] Post GSP_INIT_DONE.** Encode the element (zero authTag/aad, correct
   seqNum, elemCount, rpc header with function=GSP_INIT_DONE result=NV_OK,
   compute checkSum so the running sum is 0), write at the status-queue write
   slot, bump txHeader.writePtr.
5. **[C shell] Interrupt.** Raise the emulated MSI-X ([[mode2_interrupt_delivery]])
   so `_kgspRpcRecvPoll` wakes; also answer the GSP-RM heartbeat mailbox.

When `_kgspRpcRecvPoll` consumes GSP_INIT_DONE, `kgspWaitForRmInitDone` returns
NV_OK → `RmInitAdapter` succeeds → **the stock driver believes the GPU is live.**
That is the M3 proof-of-concept gate; M4 then triages the post-init RPC stream
and shims it into the Mode-1 core.

## Caveats / open

- **Checksum + seqNum** exact algorithm: read `GSPRPC`/`message_queue_cpu.c`
  `_gspMsgQueueCheckSum` / sequence handling before encoding (must match or the
  driver rejects the element).
- **Confidential Compute OFF** → authTag/aad unused (don't encrypt).
- **msgq ring semantics**: reuse the `msgq` library logic (`src/common/shared/msgq`)
  for pointer math rather than reimplementing — port to Rust.
- This is per-(device) state; multi-GPU keeps it per-instance.

## UPDATE (2026-06-03): the stall is PRE-mailbox

Hardware finding: across a full 6.5M-access run, the driver **never writes
NV_PGSP_FALCON_MAILBOX0/1** (0x110040/44) — only 0x110c00 (cmd queue head, ×2),
0x100c10/40 (PFB), 0x088080 (XVE/PCI-cfg mirror). So the unbounded **0xbb0080
poll happens BEFORE `kgspProgramLibosBootArgsAddr`** (which for GA106 IS the
_TU102 mailbox writer — `_f2d351` is VF/Tegra only). The driver is stuck at an
early GSP-liveness wait and never reaches the boot-args programming. Therefore
M3-step-1 (capture mailbox GPA) cannot fire yet; **0xbb0080 must be identified
and answered first.**

0xbb0000 is absent from the 575 *and* 580 public swref → computed at runtime
(likely a RISCV/PRGNLCL or engine-descriptor space). The DKMS tree
`/usr/src/nvidia-580.159.04` ships the RM core as a precompiled blob, so it
can't be instrumented. **Next step: build the fully-open 580.159.04 driver from
`/root/open-gpu-kernel-modules` (builds nv-kernel.o from source) and add a
one-shot `dump_stack()` in `kflcnRegRead_TU102`/`kflcnRiscvRegRead_TU102` when
`registerBase+offset == 0xbb0080`** — that names the function + the base (riscv
vs falcon vs other) definitively. Then answer it and resume toward the mailbox /
queue / GSP_INIT_DONE path above.

## UPDATE 2 (2026-06-03): PTIMER fix unblocked deep FWSEC progress

The 0xbb0080 spin was the **PTIMER** (GA10x relocated it to 0xbb0080/84;
constant 0 => RM timeout loops never elapse => infinite spin). Fixed by serving
qemu_clock_get_ns (commit 795be45). The driver then advances cleanly (real
timeouts) through the FWSEC/WPR bring-up. Faked, in order (all committed):
 - Falcon DMA: DMATRFCMD (GSP 0x110118, SEC2 0x840118) -> 0x2 (IDLE,!FULL) so
   s_dmaTransfer_GA102 ucode-load loops pass; SEC2 CPUCTL 0x840100 HALTED.
 - WPR2 stateful: WPR2_ADDR_LO/HI (0x1FA824/28) = 0 until the driver writes
   STARTCPU to the GSP falcon CPUCTL (FWSEC "runs"), then a region. (Driver
   checks WPR2 DOWN before FWSEC, UP after — _kgspBootGspRm + _kgspIsWpr2Initialized.)

**Current stall (NEXT):** `kgspExecuteFwsec_TU102: WPR2 initialized at an
unexpected location: 0x01000000 (expected 0xfffffe00)`. The driver computes the
*exact* expected WPR2 region from the FB layout
(kgspbuildWprMeta: frtsOffset = gspFwWprEnd - frtsSize; gspFwWprEnd derives from
FB size) and requires WPR2_ADDR_HI/LO to decode to it. Our nominal region
(0x10000000/0x10100000) doesn't match. FIX: model the emulated **FB size** (the
memory-size register the driver reads — currently 0) so its WPR computation is
deterministic, then set WPR2_ADDR_LO/HI to exactly frtsOffset/(frtsOffset+
frtsSize-1) in the WPR2_ADDR _VAL (bits 31:4, 4K-aligned) encoding. The expected
0xfffffe00 is the degenerate value when FB size reads 0; matching it (or
providing a real FB size + matching WPR2) clears this. After WPR2: Booter exec
-> GSP RISC-V boot -> the GSP message queue + GSP_INIT_DONE (the steps above).

Spike checks status: #1 GFW_BOOT ✓, #2 RISCV-enable ✓, #3/#4 falcon-halt ✓,
#5 WPR2 (in progress, exact-location), then #6/#7 GSP message queue + INIT_DONE.

## UPDATE 3 (2026-06-03): full boot faked; AT the msgq handshake (verified queue GPA)

The whole pre-GSP boot is now faked (commits through 07963a4): PTIMER, Falcon
DMA (DMATRFCMD), SEC2 CPUCTL, WPR2 (stateful, location matched via FB-size
12GiB at 0x1183a4), GSP RISC-V active (RISCV_CPUCTL 0x111388 bit7, post-FWSEC).
The stock driver reaches **GspStatusQueueInit -> msgqRxLink** and times out
(139677 polls) waiting for the GSP status-queue tx header. This is the keystone.

QEMU now reads the boot args from guest RAM (M3-step-1) AND locates the queue:
the LibOS region "RMARGS" holds GSP_ARGUMENTS_CACHED whose first struct is
MESSAGE_QUEUE_INIT_ARGUMENTS { u64 sharedMemPhysAddr@0; u32 pageTableEntryCount@8;
NvLength cmdQueueOffset@16; NvLength statQueueOffset@24 }. Verified read:
  sharedMemPA=0x139c00000  pteCount=129  cmdQOff=0x1000  statQOff=0x41000
  => status queue @ sharedMemPA + statQOff = 0x139c41000

**NEXT (the remaining keystone implementation):**
1. The shared region is page-table-described (pteCount=129): sharedMemPA points
   to a radix/PTE page table; the queue pages follow. Walk it (or, if contiguous,
   use sharedMemPA+off directly) to get the status-queue backing-store HVA.
2. Write a valid status-queue msgqTxHeader (version=0, size=statusQueueSize,
   msgSize=GSP_MSG_QUEUE_ELEMENT_SIZE_MIN, msgCount, writePtr=0, rxHdrOff,
   entryOff) via pci_dma_write so msgqRxLink links (check msgqRxLink in
   src/common/shared/msgq for the exact validated fields).
3. Decode the CPU->GSP command queue (init RPCs) and post GSP_INIT_DONE
   (rpc_init_done_v17_00, rpc_result=NV_OK) into the status queue: build the
   GSP_MSG_QUEUE_ELEMENT (zero authTag/aad, seqNum, elemCount, checkSum so the
   sum is 0), bump txHeader.writePtr, raise MSI-X.
Then kgspWaitForRmInitDone returns NV_OK and RmInitAdapter succeeds = PoC gate.

## M5 — GSP_RM_CONTROL forwarding (executable design, 2026-06-03)

State: the echo shim drives the stock driver into RmInitNvDevice; the first RPC
needing real data is GSP_RM_CONTROL (fn 76) / NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_
FALCON_INFO (cmd 0x208001b0). Its params are FLAT (numConstructedFalcons +
constructedFalconsTable[MAX], no embedded pointers) → FINN-serialized == flat, so
the response can be copied verbatim once obtained from a real GPU.

Two classes of GSP_RM_CONTROL:
1. **GPU-static subdevice controls** (falcon info, FB/GPU caps, bus info, clock
   ranges, …): the answer depends only on the silicon, not on guest-created
   objects. QEMU opens its OWN host RM client/device/subdevice (unprivileged:
   NV_ESC_RM_ALLOC root client 0x0 → NV01_DEVICE_0 → NV20_SUBDEVICE_0, then
   NV_ESC_RM_CONTROL) and reissues the SAME cmd on its own subdevice, ignoring
   the guest hClient/hObject, copying the flat response params + status=NV_OK
   into the status-queue element. Handles the bulk of init queries.
2. **Object/handle controls + GSP_RM_ALLOC** (allocations, vaspaces, channels):
   reference guest-GSP-RM handles that don't exist on the host. These need the
   guest→host handle-mapping layer — reuse the Mode-1 stub's handle translation
   ([[hclient_not_fd_scoped]], [[rmclient_validate_strict_fix]]): forward
   GSP_RM_ALLOC to create real host objects, record guest→host handle, and
   rewrite handles on subsequent controls. This is the deep long-tail.

Concrete next code step: in nvkvm_gpu_emul.c, at realize, open a host nvidia
client+subdevice (port the alloc sequence from the Mode-1 stub / src/abi/nvgpu.h
nvos21/nvos64 + NV_ESC_RM_CONTROL). In nvkvm_m3_service_cmdq, for fn==76 unwrap
{cmd,paramsSize,params@elem+? } and, if cmd is in a GPU-static allowlist, issue
NV_ESC_RM_CONTROL on the host subdevice with the flat params, copy the response
back into the echoed element (params + body.status=NV_OK), recompute checksum.
Start the allowlist with 0x208001b0; grow it per the trace. Keep QEMU
unprivileged (queries only). This is the RPC->host-RM bridge = first compute (M5).

Reference dump tool option: a standalone host C program issuing 0x208001b0 can
capture the GA106 falcon table for record/replay if live-forward setup is
deferred.

## M5 — PIVOTAL FINDING (2026-06-03): internal controls aren't userspace-forwardable

GET_CONSTRUCTED_FALCON_INFO (and the init GSP_RM_CONTROL stream) is issued by the
guest KERNEL RM on hInternalClient/hInternalSubdevice and is GSP-internal /
physical-routed (NV2080 *_GSP handler, gpu.c:5441). Such internal controls are
NOT callable from an unprivileged userspace RM client — so the Mode-1 trick
(forward the guest's USERSPACE ioctl to the host's userspace RM) does NOT apply
to these kernel↔GSP-RM controls. The GSP-RM is the authority for them and we are
impersonating it.

Consequences for M5 (the RPC long-tail). Three ways to satisfy an internal
control, by type:
1. **GPU-static internal controls** (falcon table, FB/GPU caps, engine list,
   clock domains): the answer is silicon-derived and constant. Provide it from
   chip knowledge — derive from the open driver's chip engine tables (e.g. the
   GA106 constructed-falcon set: engDesc/ctxAttr/ctxBufferSize/addrSpaceList/
   registerBase per falcon) and return a correct FLAT params struct. Bounded,
   trace-driven, per-control. Gets RmInitAdapter to COMPLETE with a real device
   model. This is the chosen incremental path.
2. **Object/alloc + dynamic controls** (GSP_RM_ALLOC, vaspace/channel/ctx, and
   controls on those objects): need a live GSP-RM. Options: (a) a privileged
   HOST KERNEL HELPER that exposes the host's real GSP-RM internal controls to
   QEMU over a side channel (QEMU stays unprivileged; the helper is a separate
   trusted host component) and forwards with guest->host handle mapping; or
   (b) reimplement the needed GSP-RM control handlers (large). (a) is the
   architecturally right path for first compute (M5/M6); it is the Mode-2
   analogue of the Mode-1 isolate but at the GSP-RPC layer.
3. **Reimplement GSP-RM** wholesale — not viable.

So Mode-2's M5 is NOT "wire in Mode-1 forwarding"; it is "impersonate GSP-RM":
static controls from chip tables (incremental, now), dynamic/object controls via
a host-kernel GSP-RM forwarding helper (next major design). The keystone (M0-M3)
+ init RPC shim (M4) are done; this is the path to first compute.

Next concrete code step (chosen path 1): hardcode the GA106 constructed-falcon
table response for cmd 0x208001b0 (numConstructedFalcons + the GA106 falcon
entries), then continue trace-driven through the next internal controls.
