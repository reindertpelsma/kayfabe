# Capturing the GSP-RM RPC boot sequence from real hardware

**Status:** step 5 complete — the recorder is **built, run on three real boards across two
architectures, and broken on purpose**. §6 = GA106, §7 = GA102, §8 = AD102 at constant driver
version. §1–§5 are the pre-build reasoning and are left as written, because the decisions they
justify are the ones that shipped.

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
- ⊘ ~~**No consumer yet.** Nothing in `crates/` reads this format; wiring it into a
  differential is separate work.~~ ⇒ **CLOSED by §9**: `tests/src/rpctrace.rs` reads the
  format and `tests/tests/replay_conformance.rs` is the conformance suite over it. Struck
  rather than deleted, because the *rest* of this list is unchanged and a reader should be
  able to see which item moved.
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

---

# 7. Step 3 — THE SECOND DIE (GA102), and what it says about "arch"

**Status: one of two boxes captured.** 2026-08-03, RTX 3090 = **GA102**, host kernel
`6.8.0-59-generic`, NVIDIA **open** kernel modules **575.51.03** rebuilt from a fresh
`open-gpu-kernel-modules` checkout at tag `575.51.03`. The Ada box (RTX 4090 = AD102) was
allocated for the same run and **never became reachable** — see §7.6.

## 7.1 The capture

`traces/ga102_boot1.bin` — committed, **1 152 928 bytes**, md5
`6bc25a2e80858c2abaa7c7bbb50ca2c8`. Summary `traces/ga102_boot1.json`, driver output
`traces/ga102_boot1_dmesg.log`.

| | GA102 (RTX 3090, 575.51.03) | GA106 (RTX 3060, 580.159.04) |
|---|---|---|
| records | **1 180** (585 req / 595 rep) | 1 076 (535 / 541) |
| payload bytes | 1 094 968 | 1 176 776 |
| largest element | **65 536** (seq 1052, `GSP_RM_CONTROL`) | 65 536 (seq 971) |
| smallest / mean | 80 / 927.9 | 80 / 1 093.7 |
| **ring wrapped?** | **no** — 1.10 MiB of a 64 MiB ring | no |
| dropped / refused-empty / rx-failed | **0 / 0 / 0** | 0 / 0 / 0 |
| NOT_SENT / len-disagree / CC | 0 / 0 / 0 | 0 / 0 / 0 |
| distinct RPC functions | 14 | 14 |
| `GSP_RM_CONTROL` elements | **724** (362 pairs) | 620 (310 pairs) |
| distinct control commands | **122** | 104 |
| **replies declaring params with no bytes** | **0** | 0 |
| sessions / retransmits within a session | 2 / **0** | 2 / 0 |

★ The boot **succeeds**: `nvidia-smi` reports the RTX 3090 on 575.51.03 and `capture.sh` exits
non-zero if it does not. The same two-session structure appears (persistence mode off ⇒ each
`nvidia-smi` is a full bring-up and teardown), and `elem_seq` restarts confirm the cut.

## 7.2 ★★★ THE CROSS-ARCH DIFF IS CONFOUNDED, AND THE CONFOUND IS MOST OF IT

The two boxes do **not** run the same driver: GA106 was captured on 580.159.04 and GA102 on
575.51.03, because 575.51.03 is what was installed on the GA102 box and a rebuilt module must
match the running userspace. ⊘ So the raw diff below is **arch ∧ driver-version**, and reading
it as an architecture result would be wrong. `ogkm is VERSIONED, not the spec`.

Raw diff (`decode_rpctrace.py --controls`, 2026-08-03): **102 common**, 20 only-GA102,
2 only-GA106, and **11 of the 102 common controls answer with a different reply size**.

Each of the three groups was then attributed **statically**, by reading both trees:

- **The 2 only-GA106 controls are pure VERSION.** `0x20800b05`
  (`INTERNAL_STATIC_KGR_GET_SM_ISSUE_THROTTLE_CTRL`) and `0x20803404`
  (`ECC_GET_REPAIR_STATUS`) are **not defined anywhere in the 575.51.03 tree**. A driver that
  has no such control cannot issue it. Nothing architectural here.

- **★★ The 11 differing reply SIZES are pure VERSION — and they are the trap.** They are the
  most die-sounding controls in the whole table — `GET_GLOBAL_SM_ORDER`,
  `GET_FLOORSWEEPING_MASKS`, `GET_PPC_MASKS` — so "the bigger die returns bigger static info"
  is the obvious reading. It is wrong, and the arithmetic settles it: these params are
  **fixed-size arrays**, so their `sizeof` cannot vary with the part.
  `NV2080_CTRL_INTERNAL_STATIC_GR_GLOBAL_SM_ORDER` gained two `NvU16` fields between the trees
  (`physicalCpcId`, `virtualTpcId`), and `NV2080_CTRL_INTERNAL_GR_MAX_GPC` went **12 → 16**:

  | control | 580 predicted | GA106 observed | 575 predicted | GA102 observed |
  |---|---|---|---|---|
  | `0x20800a22` `GET_GLOBAL_SM_ORDER` | `(9·2·240+4)·8` = **34 592** | 34 592 | `(7·2·240+4)·8` = **26 912** | 26 912 |
  | `0x20800a30` `GET_PPC_MASKS` | `16·4·8` = **512** | 512 | `12·4·8` = **384** | 384 |

  ⇒ Predicted from the headers alone, matching the wire byte-for-byte, on both trees. The
  observed size is a function of the **struct layout**, not of the GPU. ⊘ Note what this means
  for a replay: a table keyed on "arch" that stored 34 592 for `GET_GLOBAL_SM_ORDER` would be
  wrong on **the same die** under a different driver.

- **★★★ The 20 only-GA102 controls are a genuine CAPABILITY difference, and 17 of them are
  NVLink.** `INTERNAL_NVLINK_*` (11), `NVLINK_*` (4), `SYSTEM_SYNC_EXTERNAL_FABRIC_MGMT`,
  and `INTERNAL_CE_GET_HUB_PCE_MASK_V2` (the HSHUB PCE mask, which is the NVLink hub). The
  remaining 3 are `GPU_GET_RESET_STATUS`, `GPU_GET_DRAIN_AND_RESET_STATUS` and
  `PMGR_GET_MODULE_INFO`. ⊘ This is **not** version: every one of those NVLink controls is
  defined in the 580 tree too, so the GA106 driver could have issued them and did not.
  The RTX 3090 carries an NVLink connector; the RTX 3060 has none.

## 7.3 ★★★ THE SEQUENCE BRANCHES ON A REPLY, NOT ON A PART NUMBER

The cleanest single row in this whole comparison, and it needs no cross-version argument
because **both boards issue the control**:

| | GA106 (RTX 3060) | GA102 (RTX 3090) |
|---|---|---|
| `0x20800a87` `INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO` | called ×2, **status `0x56`** (`NV_ERR_NOT_SUPPORTED`) | called ×4, **status `0x0`** (`NV_OK`) |
| the 17 NVLink/fabric controls above | **never issued** | issued |

⇒ CPU-RM **probes** for NVLink with a control whose answer comes from the GPU, and the entire
NVLink sub-sequence is gated on that answer. The demand list is therefore not a property of the
architecture at all on this axis — it is a property of what the emulated GSP *replies*.

★ This is a design finding for the port, stated as a constraint rather than a conclusion: an
emulator that answers `0x20800a87` with `NV_OK` **summons 17 controls it must then serve**, and
answering it `NV_ERR_NOT_SUPPORTED` is what a GA106 really does. Refusing is both the smaller
surface and the measured-real behaviour for a part without the connector.

★★ **Corroborated by a different instrument on the same run.** The decoded RPC stream is one
reading; the driver's own `printk` is another, and `traces/ga102_boot1_dmesg.log` (2026-08-03,
RTX 3090) carries `knvlinkCoreShutdownDeviceLinks_IMPL: Need to shutdown all links unilaterally
for GPU0` **twice** — once per session — plus `nvidia-nvlink: Nvlink Core is being initialized`.
So CPU-RM on this board is genuinely driving NVLink teardown, not merely reading a mask. The
GA106 capture's dmesg has no such line. ⊘ Two instruments agreeing is worth more than either
alone (§6.2b makes the same argument), and here the second one is NVIDIA's own logging rather
than anything of ours.

## 7.4 So is the "arch" abstraction at the right granularity?

Reported either way, as required — and the honest answer is **neither of the two offered**:

- ⊘ It is **not** the case that two dies of the same generation match. They differ by 20
  controls, and that difference is real and capability-driven (§7.2, §7.3).
- ⊘ It is **also not** the case that this refutes per-generation `Arch`. Nothing in the 20 is
  *architectural* — it is one board having a connector the other lacks, discovered at runtime.

⇒ The granularity that the measurement actually argues for is **`Arch` × capability**, where
the capability is answered by the emulator and is not a property of the die name. A GA102 with
no NVLink bridge and a GA106 would, on this evidence, demand the same sequence.
⚠ **Held to what was measured:** two dies, and they differ by a *driver version* as well, so
this is one clean capability axis — not a survey. An AD102 capture is what would test whether a
generation boundary adds anything beyond capability, and it was not taken (§7.6).

## 7.5 Four instrument defects, all found by running the instrument

Recorded because in each case the tooling reported something confidently false. Nothing in this
list is in the recorder; all four are in the harness around it.

1. **The version guard refused a correct tree, on evidence it had failed to gather.**
   `build_instrumented.sh` read the running version with a `sed` anchored on 580's `/proc`
   wording (`... Kernel Module for x86_64 580.159.04`). 575.51.03 prints
   `... x86_64 Kernel Module  575.51.03` — no `for` — so the pattern matched nothing, `running`
   was the **empty string**, and the guard died with `running driver is , source tree is
   575.51.03`. ⊘ A parse failure and a genuine mismatch were **indistinguishable in its
   output**. Both readers now share one `detect_running_version`, and an unparseable `/proc` is
   its own named error.

2. **`insmod` resolves no dependencies, and the *stock* module's dependency list is the wrong
   one to read.** The load failed with `Unknown symbol ecc_make_pub_key`. The GA102 box's stock
   module is the **proprietary** one (`license: NVIDIA`), which carries its own ECC inside
   `nv-kernel.o_binary` and reports `depends:` **empty**; the module we necessarily build is the
   **open** one (`license: Dual MIT/GPL`), which links the kernel's `crypto/ecc.ko` and reports
   `depends: ecc`. Reading the installed module's dependencies would have found nothing.
   `capture.sh` now reads `modinfo -F depends` off **the module it is about to load**.

3. **★★ The restore trap cried catastrophe about an untouched bench.** A run aborted in the
   *baseline* check — step 0, before any unload — and the `EXIT` trap printed
   `‼‼‼ BENCH LEFT WITH A NON-STOCK OR NON-WORKING DRIVER — NEEDS A HUMAN`. The bench was on its
   stock module with a working `nvidia-smi` the whole time; the trap branched only on
   `RESTORED != 1`, which is equally true of every abort that happens before anything is
   swapped. ⊘ Same defect class as the empty-dmesg harness this project already names: **an
   output that does not distinguish "the thing is bad" from "I never looked"** — and here it
   failed in the direction that gets a healthy shared box torn down. The trap now branches on a
   `SWAPPED` flag set at the unload (not at the insmod, since an unload that succeeds and an
   insmod that then fails also leaves the bench off stock).

4. **The "is anything using the GPU" check fired on the script's own `nvidia-smi`.** The
   baseline `nvidia-smi -L` three lines above still held `/dev/nvidia0`, `/dev/nvidiactl` and
   `/dev/nvidia-uvm` while tearing its context down. A single instantaneous `fuser` sample
   cannot tell a departing process from a resident one; it now drains for up to 10 s first.

★ Defects 1 and 2 are the same shape and worth naming as one: **both are places where the
harness was correct for the only bench that had ever run it.** A constant that was true of
580.159.04, and a dependency list that was empty on an open-driver box. Neither is a bug in what
the recorder captures; both are the cost of a second machine, and both were silent until there
was one.

## 7.6 ⊘ The Ada box was never reachable — no AD102 capture

`ssh -p 31858 root@85.218.235.6` (RTX 4090, AD102) was polled from 2026-08-03 18:41 onward. The
**first** probe reached TCP and timed out during the SSH banner exchange; every probe after that
failed to connect at all, and the host stopped answering ICMP entirely (100% packet loss, port
closed). ⊘ Stated as what it is: **no AD102 trace was taken, and no claim about Ada is made
here.** The second architecture — the one that would actually test a generation boundary — is
still unmeasured, and §5's TU10x/AD10x/GH100 rows remain untouched apart from this GA10x
sibling. ⚠ This project's own note that a box's public address can change under a running
instance is the first thing to check if it is retried.

⇒ **SUPERSEDED by §8, and the diagnosis above was wrong.** A replacement box came up on the
same public IP on a different port and answered SSH immediately; AD102 was captured the same
day. The ICMP evidence cited here was worthless — the host filters ping — and §8.7 records how
two non-independent weak signals read as corroboration. This paragraph is left standing rather
than edited, because the retraction is the useful part.

## 7.7 What changed in the tooling, and what deliberately did not

★ **The recorder was not rewritten and was not modified.** `nv_rpctrace.c` / `nv_rpctrace.h` are
byte-identical across both captures; the record format, the ring and the `/proc` drain are
unchanged. What changed is the harness around it:

- `rpctrace-575.51.03.patch` — **`rpctrace.patch` re-anchored, not a second recorder.** Six of
  its eight hunks are the 580 patch's hunks verbatim. `patch --dry-run` of the 580 patch against
  a pristine 575 tree reports `2 out of 8 hunks FAILED`, both in `GspMsgQueueReceiveStatus`:
  580 reaches a shared `exit:` label by `goto`, and 575 has no such label and returns directly.
  The two hooks are re-expressed at **the same point in the data flow** — after
  `ccslDecryptWithRotationChecks`, after the msgLen sanity check, before the element is
  released — which is the property §2.1 actually constrains, and
  `build_instrumented.sh` re-asserts it on the post-patch text rather than trusting that
  sentence (`send@508 < encrypt@526 ; decrypt@794 < receive@860` on the 575 tree).
- `build_instrumented.sh` — version is **discovered, not constant**; `--version` / `--patch`
  override; a per-version patch is selected automatically when present; and rejects are now
  asserted absent directly, because `patch --forward` dropping a hunk yields a module that
  builds, loads, and records **half** the RPCs with no error anywhere.

---

# 8. Step 4 — AD102 AT CONSTANT DRIVER VERSION, and the confound of §7 removed

**Status: the comparison §7 could not make.** 2026-08-03, RTX 4090 = **AD102**
(`10de:2684`), host kernel `6.8.0-59-generic`, NVIDIA **open** kernel modules
**575.51.03** — the *same driver and the same kernel* as the GA102 box in §7. So
**AD102 ↔ GA102 holds driver version constant and varies only the architecture**, which is
exactly what GA106 ↔ GA102 could not do. `rpctrace-575.51.03.patch` applied unchanged, and the
§2.1 ordering property was re-asserted on the post-patch text rather than assumed
(`send@508 < encrypt@526 ; decrypt@794 < receive@860`).

## 8.1 The capture

`traces/ad102_boot1.bin` — committed, **1 140 256 bytes**, md5
`751840ae979327bf63f8833036f56507`. Summary `traces/ad102_boot1.json`, driver output
`traces/ad102_boot1_dmesg.log`.

| | AD102 (RTX 4090) | GA102 (RTX 3090) | GA106 (RTX 3060) |
|---|---|---|---|
| driver | **575.51.03** | **575.51.03** | 580.159.04 |
| records | 1 112 (551 / 561) | 1 180 (585 / 595) | 1 076 (535 / 541) |
| payload bytes | 1 085 784 | 1 094 968 | 1 176 776 |
| largest element | 65 536 (seq 1006) | 65 536 | 65 536 |
| **ring wrapped?** | **no** — 1.09 MiB of 64 MiB | no | no |
| dropped / refused-empty / rx-failed | **0 / 0 / 0** | 0 / 0 / 0 | 0 / 0 / 0 |
| NOT_SENT / len-disagree / CC | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| distinct RPC functions | 14 | 14 | 14 |
| `GSP_RM_CONTROL` elements | 656 (328 pairs) | 724 (362) | 620 (310) |
| distinct control commands | **108** | 122 | 104 |
| **replies declaring params with no bytes** | **0** | 0 | 0 |
| controls refused by a real GSP | **9** | 11 | 13 |
| sessions / retransmits within a session | 2 / **0** | 2 / 0 | 2 / 0 |

★ The boot succeeds (`nvidia-smi` reports the RTX 4090 on 575.51.03) and the stock —
**proprietary** — module was restored and verified afterwards.

## 8.2 ★★★ ZERO of 105 common controls differ in reply size

`decode_rpctrace.py --controls`, 2026-08-03, AD102 vs GA102, driver version constant:
**105 common, 17 only-GA102, 3 only-AD102, and 0 — zero — reply-size differences.**

⇒ This **confirms §7.2's static attribution empirically and from the other side.** The 11 size
differences in the GA106 ↔ GA102 comparison were predicted from header arithmetic to be pure
driver-version drift; here, across a **generation boundary** (Ampere → Ada) with the version
pinned, not one of the 105 common controls changes size by a single byte. Reply parameter sizes
on this path are a function of the **driver**, not of the die and not of the architecture.

⊘ Restated as the rule it implies for a replay table: **size is keyed on driver version, never
on arch.** A table that stored 34 592 for `GET_GLOBAL_SM_ORDER` "because GA10x" would be wrong
on the same die under a different driver and right on a *different* architecture under the same
one — which is the precise inversion of what an arch-keyed table assumes.

## 8.3 ★★★ THE 4090 REFUSES THE NVLINK PROBE — third instance, and it settles §7.3

The RTX 4090 has no NVLink connector, and the trace says so in the driver's own vocabulary:

| board | arch | driver | connector | `0x20800a87` `INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO` | the 17 NVLink/fabric controls |
|---|---|---|---|---|---|
| RTX 3060 | GA106 | 580.159.04 | none | **`0x56` `NV_ERR_NOT_SUPPORTED`** | never issued |
| RTX 3090 | GA102 | 575.51.03 | **yes** | **`0x0` `NV_OK`** | **issued** |
| RTX 4090 | AD102 | 575.51.03 | none | **`0x56` `NV_ERR_NOT_SUPPORTED`** | never issued |

⇒ The probe's answer tracks **the connector** — not the architecture (GA102 and GA106 are the
same generation and disagree; GA106 and AD102 are different generations and agree) and not the
driver version (GA102 and AD102 share a driver and disagree). Three boards, two architectures,
two driver versions, and the only variable that predicts the answer is the capability.

★★ Corroborated independently by the driver's own logging on the same runs:
`traces/ga102_boot1_dmesg.log` carries `knvlinkCoreShutdownDeviceLinks_IMPL: Need to shutdown
all links unilaterally for GPU0` **twice**, and `traces/ad102_boot1_dmesg.log` carries it
**zero** times — only the generic `nvidia-nvlink: Nvlink Core is being initialized`, which every
board emits because it is the subsystem init.

## 8.4 ★★★ AND THE ARCH DIFFERENCE THAT *IS* THERE IS ALSO A CAPABILITY — ECC

Every one of the 3 only-AD102 controls, and every one of the 3 controls whose *status* differs,
is **ECC**:

| cmd | name | GA102 (RTX 3090) | AD102 (RTX 4090) |
|---|---|---|---|
| `0x20800133` | `GPU_QUERY_ECC_CONFIGURATION` | **never issued** | ×3, `NV_OK` |
| `0x20801347` | `FB_GET_ROW_REMAPPER_HISTOGRAM` | **never issued** | ×1, `NV_OK` |
| `0x2080852b` | *(not defined in the open tree)* | **never issued** | ×1, `NV_OK`, 2 836 bytes |
| `0x2080012f` | `GPU_QUERY_ECC_STATUS` | ×1, **`0x56`** | ×2, **`0x0`** |
| `0x20800157` | `GPU_QUERY_INFOROM_ECC_SUPPORT` | ×2, **`0x56`** | ×2, **`0x0`** |
| `0x20801344` | `FB_GET_REMAPPED_ROWS` | ×1, **`0x56`** | ×1, **`0x0`** |

★★ Three more instances of the §7.3 pattern, in the opposite direction to NVLink: the **same
control**, issued by **both** boards, refused on the one lacking the capability and served on
the one having it — and the three controls only Ada issues are the follow-on that the served
answers unlock. Corroborated by a different instrument on the same runs: the `nvidia-smi` ECC
column reads **`N/A`** on the RTX 3090 (`traces/ga102_boot1_smi.txt`) and **`Off`** on the
RTX 4090 (`traces/ad102_boot1_smi.txt`) — i.e. the 4090 supports ECC and has it disabled, the
3090 has no such feature to report.

⊘ `0x2080852b` is **not defined anywhere in the open 575.51.03 or 580.159.04 SDK headers**, so
its name is unknown; it is grouped here by its position in the sequence and its 0x85 prefix
shared with the common `0x2080852c`, which is an inference and is labelled as one.

## 8.5 The last 3 controls of §7.2 close, and they close as VERSION

§7.2 attributed 17 of the 20 only-GA102-vs-GA106 controls to NVLink capability and left three
unexplained: `0x208001ab` (`GPU_GET_RESET_STATUS`), `0x208001ae`
(`GPU_GET_DRAIN_AND_RESET_STATUS`) and `0x20802609` (`PMGR_GET_MODULE_INFO`). All three are
**common to AD102 and GA102** in this constant-version comparison, i.e. both 575.51.03 boards
issue them and the 580.159.04 board does not. ⇒ **Version, not architecture.** The 20 now
resolve cleanly as **17 capability + 3 version**, with nothing left over.

## 8.6 So: is the "arch" abstraction at the right granularity?

With the confound removed, §7.4's answer holds and hardens. Across a **generation boundary** at
constant driver version, the entire observed difference is:

- **NVLink** — 17 controls, gated on a probe the GPU answers (§8.3);
- **ECC** — 6 controls, gated the same way (§8.4);
- **nothing else.** Not one control demanded merely because the die is Ada, and **not one byte**
  of reply-size difference across 105 common controls (§8.2).

⇒ **`Arch` × capability**, and on this evidence the capability term is carrying all of it. A
GA102 without an NVLink bridge and an AD102 would, on this measurement, demand the same
sequence. ⚠ **Scope, held to what ran:** three boards, two architectures, two capabilities, and
two well-behaved `nvidia-smi` boots each. No CUDA context, no compute, no refusal injection, no
reorder. This says nothing about whether the *execution* plane — where §4's data-vs-act
distinction lives — is equally capability-shaped; the controls that are **acts** are exactly the
ones a demand list cannot classify.

★ For the port, the operative consequence is a **liability**, not a feature: our emulated GSP
chooses these answers. Answering `0x20800a87` `NV_OK` summons 17 NVLink controls it must then
serve; answering the three ECC probes `NV_OK` summons three more. The answer measured on the
parts lacking each capability — RTX 3060 and RTX 4090 for NVLink, RTX 3090 for ECC, 2026-08-03,
`traces/rpctrace_ga106_boot1.bin` / `traces/ad102_boot1.bin` / `traces/ga102_boot1.bin` — is
`NV_ERR_NOT_SUPPORTED` in every case, and it is also the smaller surface: the rare place where
fidelity and least-work agree.

## 8.7 ⚠ The instrument lesson: `ping` was the wrong instrument, and it cost a box

§7.6 reported the first Ada box as unreachable on two signals: an SSH connect timeout and
**100% ICMP packet loss**. The first was real evidence. The second was **not evidence of
anything** — the host filters ICMP — and it was the one that made the diagnosis feel confirmed.
The box was destroyed on that reading and a replacement rented; the replacement came up on the
**same public IP** on a different port and answered SSH immediately. A `/dev/tcp` probe to the
port, from two different networks, is what actually discriminates, and it was not run.

⊘ This project already carries this exact trap in writing — *"refused ssh + 100% ping loss read
as a dead box; it was up"* — and it was walked into anyway. The failure was not ignorance of the
rule; it was **two weak signals reading as corroboration** because they failed at the same time.
⇒ Two instruments agreeing is only worth something when they are independent, which is the same
argument §6.2b and §8.3 make in the direction where it *works*. ICMP reachability and SSH
reachability on a filtered host are not independent — they are one signal counted twice.

★ It ended better than the correct diagnosis would have: the replacement runs **575.51.03**,
which is what made §8 a constant-version comparison at all. That is luck, and recording it as
luck is the point.

---

# 9. Step 5 — THE CONFORMANCE SUITE, and the two halves it has to have

**Status: built and green.** `tests/tests/replay_conformance.rs` (16 tests) over
`tests/src/rpctrace.rs` (the Rust reader §6.6 said did not exist). ⇒ §6.6's *"no consumer
yet"* is **closed**; every other open item there still stands.

## 9.1 The constraint, and why trace equality was never the deliverable

The owner's framing: *"hardcoding the order in a test is fine; in prod you must be protocol
compliant"* — **and** *"tests that test orders/ops the real kernel doesn't do but are still
spec compliant should pass as well."*

⊘ A test that asserts trace equality is **worse than nothing**: it pins the port to one
board, one driver and one boot, and goes red on every legitimate driver revision while
saying nothing about compliance. So there are two halves, and the second is not garnish —
without it the first is trace lock-in wearing a test's clothes.

| half | what it does |
|---|---|
| **replay** | re-issues the recorded control demand through the real `msgq` transport at the real `served_policy()` chain, substituting only what varies per boot, and judges the *answers* by protocol properties |
| **reorder** | feeds five sequences a real kernel never issues but which are spec compliant under a declared order model, and requires them to **pass** |

Nothing in the file asserts *"the Nth element must be X"*.

## 9.2 What the three captures prove, now as tests

`[measured]` 2026-08-03 by `cargo test -p kayfabe-tests --test replay_conformance` over
`traces/rpctrace_ga106_boot1.bin`, `traces/ga102_boot1.bin`, `traces/ad102_boot1.bin`.

1. **Reply size is keyed on driver VERSION, not architecture.** 0 of **105** common
   controls differ across the Ampere→Ada boundary at constant driver. ★ And the *converse*
   is now measured too, which §8.2 did not state: the set of ids that moves across the
   version boundary is **identical** (11 ids) whether the comparison also crosses a
   generation (GA106↔AD102) or not (GA106↔GA102). "Keyed on version" is therefore not an
   inference from a zero — it is a positive, matching set.
2. **The sequence branches on a REPLY.** Encoded as a **biconditional** over three boards:
   each of the 17 NVLink controls appears **iff** `0x20800a87` answered `NV_OK`, and each of
   the 3 ECC controls **iff** all three ECC probes did. Both directions bite (a dependent
   without its probe, and a served probe without its closure), and the check refuses to pass
   vacuously if all three boards agree.
3. **A reply is a pure function of `(cmd, request params)`** — over 163 / 185 / 184 distinct
   argument keys, the number of keys with more than one answer is **one**, `0x20801819`
   (live PCIe counters), the same on all three boards. ⇒ a **params-keyed** table is
   expressive enough for the demand list; a cmd-id-keyed one is not.
4. **Refusal is ordinary protocol behaviour** — 13 / 11 / 9 controls answered non-`NV_OK` by
   real firmware on boots that reach a working `nvidia-smi`, exactly two of them conditional.

## 9.3 ★★★ Two protocol facts the earlier sections did not have

- **`paramsSize` is NOT bounded by the element.** `0x2080a0a4` (GSS-legacy, defined in
  neither open tree) declares `paramsSize = 67396` inside an element of exactly
  `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX = 65536` — **65 416 present against 67 396 declared**, in
  both directions, on all three boards, answered `NV_OK`. This is the *inverse* of the
  `dlen = 0` class: not an absent measurement but a present one saying the declaration
  overruns the message (`[measured]` 2026-08-03,
  `tests/tests/replay_conformance.rs::params_size_is_not_bounded_by_the_element_and_real_firmware_answers_anyway`,
  over all three committed captures). ⇒ an emulator may not treat `paramsSize > delivered`
  as malformed, and must clamp to what arrived.
- **The per-boot substitution surface is `hClient`, and it is measured.** Across the two
  bring-ups in each capture the `hClient` sets are disjoint apart from one persistent
  RM-internal client (`0xc2000006`), while the `hObject` sets are equal apart from a single
  entry. ⇒ a replay substitutes client handles and **nothing else** — and ⊘ it must not
  pretend to substitute a live counter.

## 9.4 The order model, and the fork it resolves

A permutation is admitted only if it moves controls the static pass classified **`DATA`**
(`docs/reference/gsp_control_classification.tsv`). `ACT`, `MIXED` and — deliberately —
`UNKNOWN` keep their positions: **absence of a classification is not a licence**, the same
rule as the FIFTH LIMIT's *"an empty capture is evidence of NOTHING"*.

⚠ **This is the design fork the task named, and it is resolved narrowly rather than
invented around.** A *general* spec-compliant reordering needs a full inter-control
dependency model, which this project does not have. What it has is a per-control, cited,
order-independence claim for 106 controls, and that is the universe the model quantifies
over. Widening it later means widening the classification, not editing the test.

Five sequences, all green: the `DATA` sub-sequence reversed, rotated by a third, shuffled
under two seeds, every `DATA` control issued twice, and the two bring-ups interleaved. What
is asserted is that the port is a **function of the request** — the same `(cmd, params)`
gets the same answer in every ordering — never that the reply *streams* match.

## 9.5 ★★★ What the suite FOUND, which is the part that mattered

`0x20800301` `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` is a control this port **claims**, is
answered `NV_OK` by real GA106 firmware on all six of its recorded calls, and is answered by
this port **`NV_OK` once and `NV_ERR_NOT_SUPPORTED` five times**. Two different causes hide
behind one id:

1. Four refusals are `kayfabe_abi::eventnotify::SILENT_NOTIFIERS` — deliberate and argued:
   accepting a registration is a promise to deliver, this device delivers nothing, so only
   notifier indices whose silence is *true* of this device may be accepted.
2. ★ The fifth is **not** deliberate. `InitTablePolicy::notify_actions` outlives the guest
   driver lifetime that the guest's own `Subdevice` does not, so after the guest's `rmmod`
   the already-armed transition rule fires against a guest that is legitimately re-arming.
   `a_guest_teardown_does_not_reset_this_port_s_notifier_state` isolates it away from the
   replay harness (arm → fn 47 → arm), so the finding does not rest on the harness
   collapsing two bring-ups into one transport.

⊘ **Recorded and pinned, not fixed here.** Whether fn-47 should reset this port's
event-plane state is a decision about state lifetime across guest driver lifetimes.

★ And note the shape: this refusal is returned by a link that **claimed** the command, so it
never reaches `unserviced::UnservicedLedger` and **diffing ledgers cannot find it** — the
project already carries that exact trap in writing. A test that judges the *answer* can.
That is why the claimed-but-refused set is a pinned list with a reason per row, not a
predicate.

## 9.6 What is green, and what it cost

- 24 of 24 `WantedTable` entries are demanded by the GA106 capture, and **every one of their
  reply `paramsSize` values agrees with real GA106 GSP firmware on 580.159.04.**
- 84 of the 310 recorded control calls are ones this port claims; the other 226 are refused
  with `NV_ERR_NOT_SUPPORTED` and **zero bytes of the guest's own request coming back** —
  125 of them carried non-zero `[in]` params an echo would have handed straight back, which
  is what stops that assertion being vacuous.
- Every property is a function that was **seen to fail**: 17 reader mutations refused, plus
  a mutation per protocol property, plus two policy mutants (an echo-everything policy and a
  position-dependent one). ⊘ Two mutations are **inert and listed rather than dropped** — a
  payload byte flip is invisible to a structural reader, and perturbing a reply for an
  argument key that occurs once settles nothing.

## 9.7 ⊘ What this suite does NOT establish

- **One driver version is replayed.** The GA106 capture is 580.159.04 = `BENCH_DRIVER`; the
  other two are 575.51.03, and this port selects a different wire table for them, so
  replaying their bytes at this ABI would measure a version mismatch rather than
  conformance. They are used in §9.2, which is version-aware by construction.
- **`GSP_RM_CONTROL` only.** The other 13 RPC functions in the captures are not replayed.
- **Well-behaved boots only** (§6.6's limit stands): no CUDA context, no compute, and no
  *deliberate refusal* injected at the guest — the reorder half authors new **orders**, not
  new **failures**.
- **Observational neutrality is still not proven** (§6.6), and the port's answers are judged
  against a recorder that sits inside CPU-RM.
