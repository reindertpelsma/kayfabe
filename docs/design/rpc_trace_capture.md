# Capturing the GSP-RM RPC boot sequence from real hardware

**Status:** step 1 complete — the existing driver levers were checked and are **insufficient**;
the hook points are located and named. Nothing has been built or run yet.

## 0. Why this exists, and what it replaces

Our best hardware oracle today is the C artifact's captured control table,
`C: src/qemu/mode2_initctrl_ga106.h` — **56 rows, of which 11 (19.6%) carry `dlen = 0`**, i.e.
the reply body was never captured. Every empty row checked against a real GA106 is
**contradicted**; `0x20802a08` (`CE_GET_FAULT_METHOD_BUFFER_SIZE`) decodes from its empty row
as size 0 where hardware answers 20480, and RM DMAs CE fault records into a buffer of exactly
that size. That table is not merely incomplete — on those rows it is **positively wrong**, and
the project's standing rule is `⊘ an empty capture is evidence of NOTHING, not evidence of
emptiness` (`../nvidia-gpu-passthrough/CLAUDE.md`, the FIFTH LIMIT).

A recorder that sits **inside CPU-RM**, at the point where the complete message-queue element
exists, cannot produce an empty row: it has the bytes or it has nothing at all. So this is not
only a faster iteration loop — it is the replacement for the one part of our evidence base that
is known to be wrong.

★ Second, and larger: `traces/mode2_c_reference/cap1_coldboot_hermetic` is a trace of a boot
that **fails** — it ends where our emulator stopped. A bare-metal capture is a trace of a boot
that **succeeds**. It carries the whole demand sequence through to a working `nvidia-smi`
*and* the replies a real GSP gave. That is categorically more information than anything the
project currently owns.

## 1. ⊘ What the driver ALREADY has, and why none of it is enough

This section exists because the previous three instruments this project built were either
already present elsewhere or were reading the wrong artifact. **Checked first, built second.**

### 1.1 The RPC history ring — real, and far too small

`ogkm-580: src/nvidia/inc/kernel/gpu/rpc/objrpc.h:56,60-66,89-92` — `OBJRPC` carries
`rpcHistory[RPC_HISTORY_DEPTH]` and `rpcEventHistory[RPC_HISTORY_DEPTH]` with
`RPC_HISTORY_DEPTH == 128`, and each entry is

```c
typedef struct RpcHistoryEntry {
    NvU32 function;
    NvU32 sequence;
    NvU64 data[2];
    NvU64 ts_start;
    NvU64 ts_end;
} RpcHistoryEntry;
```

⇒ **function number, sequence, sixteen bytes, two timestamps.** No request body, no reply body,
and a 128-entry ring that wraps many times over during a boot. It is a crash-diagnostic tail,
which is what it was built to be (`kernel_gsp.c:1879-1886` walks it to attribute a timeout).
It cannot answer *"what did GSP reply to `0x20800a36`"*, which is the only question we have.

### 1.2 `NV_PRINTF` on the RPC path — status lines, not payloads

Every `NV_PRINTF` in `ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c` around the RPC
path reports *conditions* — `"GSP crashed, skipping RPC"`, `"received event from GPU%d: 0x%x
(%s) status: 0x%x size: %d"` — never bodies. Raising `NVreg_ResmanDebugLevel` makes these
appear; it cannot make them contain what they do not print.

### 1.3 `RmMsg` — a filter, and debug-build only

`ogkm-580: src/nvidia/src/kernel/rmapi/client_resource.c:1304-1310` and
`ctrl0000system.h:626-638`. `RmMsg` selects which existing prints fire, and the source says in
its own comment *"RmMsg is only available when NV_PRINTF_STRINGS_ALLOWED is true"*, i.e. debug
builds. A filter over prints that do not carry payloads still yields no payloads.

**Conclusion:** there is no existing lever that dumps request/reply bodies. The recorder has to
be written. ⊘ That is a *finding*, not a foregone conclusion — it took ~30 minutes and it
stopped us guessing.

## 2. Where to hook — two functions, and both have the whole element

`ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c`:

| direction | function | line | called from |
|---|---|---|---|
| CPU → GSP | `GspMsgQueueSendCommand(MESSAGE_QUEUE_INFO *, OBJGPU *)` | `:456` | `kernel_gsp.c:411` |
| GSP → CPU | `GspMsgQueueReceiveStatus(MESSAGE_QUEUE_INFO *, OBJGPU *)` | `:608` | `kernel_gsp.c:1770` |

The send path opens with the complete element and its exact length already computed:

```c
GSP_MSG_QUEUE_ELEMENT *pCQE = pMQI->pCmdQueueElement;
NvU32 msgLen = GSP_MSG_QUEUE_ELEMENT_HDR_SIZE + pMQI->pCmdQueueElement->rpc.length;
```

So a record is `(direction, seqNum, timestamp, msgLen, bytes[msgLen])` and needs no parsing,
no reassembly, and no knowledge of any particular control. **Two call sites. That is the whole
instrumentation surface.**

### 2.1 ★★★ The ordering trap that would silently record garbage

`GspMsgQueueSendCommand` **encrypts the element in place** when Confidential Compute is on
(`message_queue_cpu.c:485-497`, `ccslEncryptWithRotationChecks(... pSrc + HDR, ... pSrc + HDR
...)` — same buffer in and out). A hook placed after that block records **ciphertext**, and
would do so without any error, on exactly the machines where the capture is hardest to repeat.

⇒ The send hook goes **before** the CC block; the receive hook goes **after** decryption. Our
target is CC-off (`mode2_rewrite_design_decisions`), so on today's benches the two placements
would produce identical bytes — which is precisely why the wrong one would go unnoticed until
somebody captured on a CC part. Same shape as every instrument defect this project has found:
correct on the machine you tested, silently wrong on the machine that mattered.

## 3. Why `printk` cannot carry this

⊘ **Do not route this through `dmesg`.** `cap1_coldboot_hermetic` holds 359,062 records for a
boot that did not even finish, and the kernel ring buffer drops without telling the reader
which lines went. A capture with holes is worse than no capture, because the holes are exactly
where a `diff` would have been positional (`nvkvm_m2_rec` carries the same rule: *never sample
or cap*).

Design instead: a `vmalloc` ring inside the module, fixed-capacity, **binary** records, drained
through `procfs`/`debugfs` after the boot completes, with an explicit **overflow counter that
the dump refuses to omit**. If the ring wrapped, the consumer must be told so it can refuse the
trace rather than replay a hole.

## 4. What this buys, and what it does NOT

★ It answers **demand** — the ordered sequence of RPCs the driver issues, with the replies a
real GSP gave — for a boot that reaches a working `nvidia-smi`.

⊘ It does **not** by itself tell us which of those RPCs are *data* (a reply is the whole answer)
and which are *acts* (the reply is an acknowledgement of something that must actually happen).
That distinction is the difference between a control we can serve from a table and one we must
perform, and this project has already met two of the latter — `0x20800a6c`
(`INTERNAL_MEMSYS_L2_INVALIDATE_EVICT`, #148) and `0xa06f0103` (`GPFIFO_SCHEDULE`, #177), both
all-`[IN]`. **A replay that answers an act from a table fails LATE**, as a hang or wrong data
hundreds of RPCs later, not at the control.
⇒ So a trace enumerates the demand list; classifying each entry data-vs-act is a separate,
*static* pass over `ogkm`, and it is the pass that keeps a replay honest. The C's own history is
the warning: its echo-by-default reached a boot quickly and then produced a two-day `rbp`-clobber
SIGSEGV that was precisely an act/data confusion surfacing late
(`C: src/qemu/nvkvm_gpu_emul.c:2954` and the M5.3/M7/M8/M8.1 comment chain above it).

⊘ These are traces of **successful, well-behaved** boots. They are a strong positive oracle and
a weak negative one: they say nothing about what the driver does when refused, when something
arrives out of order, or when it arrives late. Our port's posture is refusal-first, so
deliberate-refusal and spec-compliant-reorder cases have to be authored, not recorded.

⊘ Only the **open** driver can be instrumented. The port targets closed and open
(`multi_driver_support`). GSP-RM is the same firmware and CPU-RM is shared source, so the
sequences *should* correspond — but "should" is not measured, and this limit is recorded rather
than assumed.

## 5. Planned scope

Capture is independent of implementation and far cheaper, so it comes first and in bulk.

| generation | example part | why |
|---|---|---|
| TU10x | RTX 2080 | oldest GSP-capable HAL (`kernel_gsp_tu102.c`), cheapest box |
| GA10x | RTX 3060 | the reference chip; the arm the C oracle already covers |
| AD10x | RTX 4090 | nearest neighbour to GA10x — the first second-arch we would implement |
| GH100 | H100 | the arch-seam stress case; optional, and the only costly line item |

⚠ **Capturing is not implementing.** `Ad10xArch` and `Gh100Arch` currently delegate `mmu`,
`userd`, `pushbuffer` and `is_case2_control` to `kayfabe_mocks::MockArch`, so a multi-arch
conformance test written today would be measuring mocks against mocks — the exact defect
`mock_fidelity_both_directions` names, twice caught this week. Trace first; implement the second
arch properly; hold the rest.
