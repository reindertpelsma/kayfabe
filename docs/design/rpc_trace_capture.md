# Capturing the GSP-RM RPC boot sequence from real hardware

**Status:** step 2 complete — the recorder is **built, run on a real GA106, and broken on
purpose**. §6 has the results, the numbers and the limits. §1–§5 are the pre-build reasoning
and are left as written, because the decisions they justify are the ones that shipped.

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

---

# 6. Step 2 — BUILT, RUN ON REAL HARDWARE, AND BROKEN ON PURPOSE

**Status: complete.** 2026-08-03, `vb` (vast.ai instance 46494693), RTX 3060 = **GA106**,
host kernel `6.8.0-59-generic`, **NVIDIA open kernel modules 580.159.04** rebuilt from
`research_clones/ogkm-580.159.04` (`version.mk: 580.159.04`, git tag `b81d58e`).
Everything below is `scripts/rpctrace/`.

| file | what it is |
|---|---|
| `nv_rpctrace.h` | the record format, and the three hook prototypes |
| `nv_rpctrace.c` | the recorder: vmalloc ring + `/proc/driver/nvidia/rpctrace` |
| `rpctrace.patch` | the hooks — 2 capture points, 3 accounting calls, nothing else |
| `build_instrumented.sh` | pristine tree → instrumented `nvidia.ko`, with the placement assertion |
| `capture.sh` | swap in, boot, drain, **restore, and verify the restore** |
| `decode_rpctrace.py` | verify-or-refuse, summary, `--list`, `--controls`, `--hexdump` |
| `test_decoder_refusals.py` | mutate the real capture, watch the guards fire |
| `rpc_function_names.tsv` | @generated name map (262 entries) from `rpc_global_enums.h` |

## 6.1 The capture

`traces/rpctrace_ga106_boot1.bin` — committed, **1 229 472 bytes**, md5
`0fcc24c7074df68a585868b75326f329`. Summary in `traces/rpctrace_ga106_boot1.json`,
driver output in `traces/rpctrace_ga106_boot1_dmesg.log`.

| | |
|---|---|
| records | **1 076** (535 CPU→GSP, 541 GSP→CPU) |
| payload bytes | 1 176 776 — every one of them present |
| largest single element | **65 536** = `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` exactly (seq 971, `GSP_RM_CONTROL`) |
| smallest / mean | 80 / 1 093.7 bytes |
| **ring wrapped?** | **no** — 1.17 MiB used of a 64 MiB ring; `n_dropped = 0` |
| dropped / refused-empty / rx failures | **0 / 0 / 0** |
| distinct RPC functions | 14 |
| span | 3 701 ms |

★ **The boot SUCCEEDS.** `nvidia-smi` reports the RTX 3060 with driver 580.159.04 and
`capture.sh` exits non-zero if it does not. That is the whole difference from `cap1`.

★★ **It contains TWO complete sessions, and the first reading of that was wrong.** Every
per-function count came out even and 479 records shared an `elem_seq`; read naively that is
479 retransmits. It is not. With persistence mode off RM tears the GPU down when the last
client closes, so each `nvidia-smi` invocation is a full bring-up *and* shutdown and the
message queue's `seqNum` restarts at 0. `decode_rpctrace.py` now cuts sessions where a
direction's `elem_seq` goes backwards and reports retransmits **within** a session:
**0**. ⇒ Sessions #0 (479 records, 1 659 ms, 13 functions) and #1 (597 records, 1 557 ms,
14 functions) are two independent bring-ups of the same GPU in one file — which makes the
trace a repeatability check as well as a capture.

## 6.2 ★★★ What it answers that `mode2_initctrl_ga106.h` got WRONG

`decode_rpctrace.py --controls` decodes the `GSP_RM_CONTROL` elements: **620 elements, 310
request/reply pairs, 104 distinct control commands**, and

> **replies declaring params with no bytes present: 0.**

All six of the empty rows the FIFTH LIMIT names as *contradicted by hardware* are here with
bodies, and the headline one reproduces the independent 2026-08-01 measurement exactly:

| cmd | C table | this trace | decoded |
|---|---|---|---|
| `0x20802a08` `CE_GET_FAULT_METHOD_BUFFER_SIZE` | `dlen=0` ⇒ 0 | 4 bytes ×10 | **20480** |
| `0x20802a06` | `dlen=0` | 4 bytes ×4 | 16 |
| `0x2080017e` | `dlen=0` | 8 bytes ×2 | 33554432 |
| `0x20800af3` | `dlen=0` | 2 bytes ×4 | `0101` |
| `0x20800a4b` | `dlen=0` | 4 bytes ×2 | 67174400 |
| `0x20800aac` | `dlen=0` | 4 bytes ×2 | 65536 |

⇒ The 20480 was previously established by a *different* instrument (a `NV_PRINTF` probe,
`traces/real_ga106/fmb_real_ga106.txt`). Two independent instruments agreeing on it is worth
more than either alone, and it is the cross-check that says this recorder is reading the right
bytes.

★ **A genuinely zero-length reply is now DISTINGUISHABLE from an unmeasured one**, which was
the whole complaint. `0x20800a70` answers with `paramsSize = 0` and status `NV_OK`; that is a
measurement — the element was captured entire and it says zero. An empty row in the C table
was the *absence* of a measurement wearing the same clothes.

⊘ **And this still does not classify data-vs-act** (§4). `0x20800a6c`
(`INTERNAL_MEMSYS_L2_INVALIDATE_EVICT`, #148) appears 8 times and answers **17** on some calls
and **49** on others; `0xa06f0103` (`GPFIFO_SCHEDULE`, #177) answers 3 bytes. Both look exactly
like data in this table. A replay that served either from it would fail late.

## 6.2b ★★ The decoder checked against an instrument that is not itself

A decoder that mis-locates a field produces a table that is wrong in a completely
self-consistent way, so it cannot be caught by reading its own output. `traces/real_ga106/
rpc_transcript_real_ga106.txt` is an **independent** measurement of the same GPU by a different
instrument — an `NV_PRINTF` probe in `rpcRmApiControl_GSP`, taken 2026-08-01 — and it prints
`cmd`, `psize` and `gspst` for 88 control calls.

Compared against this capture's decoded `GSP_RM_CONTROL` replies (`[measured]` 2026-08-03,
`traces/rpctrace_ga106_boot1.bin`):

| | |
|---|---|
| transcript lines | 88 |
| agree on **both** `paramsSize` and GSP status | **88** |
| disagree | **0** |
| commands absent from the new trace | **0** |

⇒ The element-header (48) and RPC-header (32) offsets `--controls` decodes through are the
right ones, and the status field is where it is claimed to be. This is worth more than the
agreement on `0x20802a08` alone, because it covers 88 calls including the `0x56` refusals.

★ The new capture is a strict superset: 104 distinct commands against the transcript's smaller
set, **and it carries the reply bodies**, which the probe never printed.

## 6.3 Breaking it on purpose

**The ring, on real hardware.** Re-captured with `--kb 512`, i.e. a 512 KiB ring against a boot
that needs ~1.2 MB. The recorder filled, refused **262 records / 821 760 bytes**, and set
`NV_RPCTRACE_FF_OVERFLOWED`; the decoder exits **2** with `RING OVERFLOWED … The trace is a
PREFIX, not a capture.` This is the guard firing on a real overflow produced by a real driver,
not on a hand-edited header.

**The file.** `test_decoder_refusals.py` mutates the **real capture** — not a synthetic fixture,
so it tests the decoder against the format the *recorder* produces — and asserts the clean file
is accepted first, because a suite whose subject is refused for an unrelated reason scores
perfectly having tested nothing. 13 mutations refused: truncate by 1 byte, truncate by 1000,
truncate inside the file header, trailing garbage, zeroed file magic, bumped version, wrong
record-header size, claimed `n_dropped`, claimed `n_rx_failed`, corrupted mid-stream record
magic, `cap_len = 0`, a gap in the record counter, and a header record count off by one.

⊘ **One mutation is INERT and is listed rather than dropped:** flipping a byte inside a
record's payload changes the file and changes nothing the decoder can see. There is no
integrity check over bodies and there cannot be a useful one — the recorder copies them out of
driver memory with nothing to compare against. So "13 of 14 caught" must not be read as "the
decoder validates payloads". It validates *structure*.

## 6.4 Two instrument defects found by running the instrument

Both are recorded because in each case the instrument reported something confidently false.

1. **The placement gate matched its own documentation.** `build_instrumented.sh` asserts the
   send hook precedes the encrypt call. Anchored on the bare name `ccslEncryptWithRotationChecks`,
   it found the occurrence inside the *hook's own comment* — three lines above the hook — and
   failed the build on a correctly-placed hook. Anchoring on `= ccsl…` and on
   `nv_rpctrace_record(` matches code and only code.
2. **`nm … | grep -q` under `pipefail` is a false red, and the line below it was a false green.**
   `grep -q` exits at the first match, `nm` on a 100k-symbol module dies of SIGPIPE, `pipefail`
   fails the pipeline — so the check reported "symbol not in the module" about a module the
   symbol was in. The identical construct one line down passed only because `modinfo -p` is
   short enough to finish first, i.e. for a reason unrelated to the parameter existing. Both go
   through a file now.

## 6.5 ⚠ The bench, and one thing that went wrong on it

`vb`'s stock module is **never modified on disk**: the instrumented one is `insmod`ed by path
and the restore is a plain `modprobe`. The restore is verified by two positive discriminators
(`/proc/driver/nvidia/rpctrace` must be gone, and `/sys/module/nvidia/srcversion` must equal the
DKMS module's) plus a working `nvidia-smi` — `scripts/rpctrace/capture.sh`, `restore_stock()`.
**Observed on `vb`, 2026-08-03:** the final run printed `srcversion matches the stock DKMS
module (EF35EC2DD7E7BD18B01732F)` and `GPU 0: NVIDIA GeForce RTX 3060`, and the same two checks
are re-run by the `EXIT` trap on every failure path.

★ **MEASURED on `vb`, 2026-08-03, first run of `capture.sh`: that check FAILED and the bench was
left on the instrumented module.** `nvidia-smi` shells out to `nvidia-modprobe`, which loads
**`nvidia_uvm`** — so by drain time `/sys/module/nvidia/refcnt` read `1` with
`/sys/module/nvidia/holders/nvidia_uvm`, and `rmmod nvidia` failed. The restore path had only
ever considered `nvidia` itself. The bench was restored by hand within a minute (`rmmod
nvidia_uvm; rmmod nvidia; modprobe nvidia`, then `srcversion` back to `EF35EC…` and
`nvidia-smi -L` working), and both the pre-load and the restore now go through one
`unload_all`. The check earned its keep on its first outing: a restore verified only by
"modprobe returned 0" would have reported success on that run.

## 6.6 What is still open

- ⊘ **One part, one kernel, one driver.** GA10x only; §5's TU10x/AD10x/GH100 rows are untouched.
- ⊘ **Open driver only** (§4's last limit stands): the closed driver cannot be instrumented, and
  that the sequences correspond is still *assumed*.
- ⊘ **Only a well-behaved boot.** `nvidia-smi`, twice. No CUDA context, no compute, no refusal
  and no reorder — those have to be authored, not recorded.
- ⊘ **Data-vs-act is not answered** (§4, and §6.2's last paragraph). That pass is static, over
  `ogkm`, and it is the one that keeps a replay honest.
- ⊘ **No consumer yet.** Nothing in `crates/` reads this format; wiring it into a differential
  is separate work.
- ⊘ **Observational neutrality is NOT proven.** The hooks add a `memcpy` of up to 64 KiB under a
  spinlock on the RPC path, and this project's own rule is that a recorder can perturb what it
  records (`nvkvm_m2_rec` is *not* observationally neutral, which is why `m2_trace` must never be
  reused for capture). The evidence here is indirect and is worth exactly what it is: the 88/88
  agreement in §6.2b is against a capture taken from a **differently instrumented build** of the
  same driver, so the control sequence and its sizes are stable across at least those two builds.
  That is not the same as showing the sequence is what an *uninstrumented* driver issues, and no
  measurement in this task establishes that.
- ⊘ **Two sessions by construction, not by design.** `capture.sh` runs `nvidia-smi` twice, so the
  file holds two bring-ups. That is useful (it is a repeatability check) but a consumer wanting a
  single boot must cut at the session boundary; the decoder reports it, nothing enforces it.
