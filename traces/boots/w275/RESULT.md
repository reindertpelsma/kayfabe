# w275 — RESULT: the divergence record untruncated, and both `0x56` measured INERT

**STATUS: LIVE — 2026-08-12.** One boot (`w275_pin`), build rev **`55c5d16`**, STAMP GATE PASS,
**all six ARM ASSERTIONS PASS**. One native host reference (578×2, noise floor **zero**). One
native fault-injection matrix on a real GA106. Evidence in this directory,
`nvidia-gpu-passthrough@4bf4dd0:traces/nvdiff_w275/`, and
`traces/real_ga106/ctxcreate_fault_injection_matrix.txt`.

---

## ★★★★★ LEAD: THREE THINGS CONTRADICT THE BRIEF, AND TWO OF THEM ARE "ALREADY BUILT"

### 1. HALF B's answer was already committed — on an UNMERGED branch nobody would find

`origin/status-divergence` (`28cf456`, `d91f44d`, 2026-08-10, **1535 insertions across 29
files**) already serves **both** controls, with tests. It is **not** an ancestor of w274's
`ba2927b`, not in `master`, and reachable from exactly one ref:

```
$ git branch -a --contains 28cf456
  remotes/origin/status-divergence
```

⇒ Nineteenth consecutive lane whose premise was already built. ★ The standing instruction
(`git log --all --grep` before dispatch) **worked** — but only because it quantifies over `--all`;
a branch-local search would have missed it, and the branch name does not contain "0x56", "binapi"
or "perfboost".

### 2. The brief's second id is REFUTED as an id this port ever sees

`28cf456` measured, and I re-confirmed on this branch: **`0x2080200a` never reaches the device.**
`grep -l 0x2080200a traces/guest_boots/*_qemu.log` → **0 files**; `0x20800a9a` → **95**. The guest
kernel implements `NV2080_CTRL_CMD_PERF_BOOST` itself and `kperfBoostSet_IMPL` re-packages it as
`NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X` (`0x20800a9a`) for physical RM
(`ogkm-580.159.04/src/nvidia/src/kernel/gpu/perf/kern_perf_boost.c:84-92`), returning that status
unchanged. ⇒ The `0x56` the differential sees on `0x2080200a` **is our refusal of `0x20800a9a`,
one translation later.** The userspace census and the device census were watching **one event
through two boundaries and calling it two ids.**

### 3. HALF A's "per-command size table" was already built, and the cap was the ONLY bug

`uvm_sizes.h` is generated from the driver headers by `gen_uvm_sizes.sh` and already carried
`MAP_EXTERNAL_ALLOCATION = 9264`. ⊘ And the brief's warning that this struct "carries a
variable-length array" is **wrong**: `perGpuAttributes[UVM_MAX_GPUS]` is a **fixed-size** array,
`sizeof` is a compile-time constant, and 9264 is the largest entry in the whole table. Nothing had
to be derived. The recorder was losing exactly **1072 bytes** to an 8192 cap.

★ **The default was the bug, not the mechanism.** Three orchestrators
(`nvdiff_run_guest.sh`, `nvdiff_orch_bench.sh`, `nvdiff_capture_cycle.sh`) **already pinned
65536**, each with a comment saying why. w274's hook drove `nvd_capture.sh` directly and silently
got 8192. A correct value carried by every caller *except* the default is not a default.

---

## HALF A — WHAT DIFFERS INSIDE THE UNTRUNCATED RECORD

**Nothing that means anything.** Re-captured both arms at 65536: **0 truncated records** on either
side (against 21 guest / 28 host before), all 25 host and 18 guest
`UVM_MAP_EXTERNAL_ALLOCATION` records at the full 9264 bytes.

### ★★★★★ The 1072 recovered bytes contain ZERO divergences

That call still shows **exactly 36** UNEXPLAINED — identical to the count computed over partial
buffers. Offsets 8192..9263 are **byte-identical** between host and guest in all 18 comparable
records.

### ★★★ And the 36 are ONE fact repeated eighteen times: the GPU UUID

Every one is at offset **`0x18`**, with the same 16-byte pair in every record:

```
host   d09136851ec0805ae31943a901a0e1ff
guest  78b352c71ccd7a86d28249484c827f27
```

`0x18` is `perGpuAttributes[0].gpuUuid` — `base`(8)+`length`(8)+`offset`(8) = 24 = 0x18, and
`UvmGpuMappingAttributes` opens with a 16-byte `NvProcessorUuid`
(`ogkm-580.159.04/kernel-open/nvidia-uvm/uvm_ioctl.h:493-497`).

It is **environmental**, and self-consistency is measured, not assumed:

| | carries its OWN uuid | carries the OTHER side's |
|---|---|---|
| guest | **80** record-fields | **0** |
| host | **96** record-fields | **0** |

The guest registered a GPU under that UUID and referred to it consistently across
`REGISTER_GPU`, `REGISTER_GPU_VASPACE`, `REGISTER_CHANNEL` (32), `MAP_EXTERNAL_ALLOCATION` (36),
`ALLOC_SEMAPHORE_POOL`, `MAP_DYNAMIC_PARALLELISM_REGION`. **Zero cross-contamination.**

⇒ **80 of the 132 value divergences (60.6 %) are that single identity constant** — the `CARD_INFO`
class. The genuinely unexplained remainder is small and nameable: `0xc36f0108` 16, `0x00800292` 12
(one contiguous run at `0x188`..`0x1ac`), `GPU_GET_NAME_STRING` 2, `CARD_INFO` 2, all others ≤3.

⇒ **The call at the divergence point does not differ in content.** The guest issues it with
semantically identical parameters and stops after 18 of 25.

### Census, ranked BY KIND — reproduces w274 within one

`A(host)=578 B(guest)=436 ratio=0.710` → **428** divergences (w274: 429).
EXTRA **76** (w274: 77 — every one `0x20801702`), MISSING 218, UNEXPLAINED 132, STATUS 2 at the
same two indices `A[41]`/`A[95]`.

---

## HALF B — BOTH `0x56` ARE MEASURED INERT FOR `cuCtxCreate`, ON REAL HARDWARE

### The mechanism: UNHANDLED FALLTHROUGH, not a blanket refusal

Determined from source, not assumed. `crates/kayfabe-gsp/src/boot.rs:1520-1532`: when **no policy
link claims** a command, the loop records it and mints the status itself —

```rust
None => { report.unserviced.push(...); cmd.reply(NV_ERR_NOT_SUPPORTED, &[]) }
```

`InitTablePolicy` declines by `WantedTable::from_cmd(req.cmd)?` (`inittables.rs:1380`) — a lookup
miss, not a refusal; `kayfabe_rmrpc::ObjectPolicy` gates on a 5-id list that excludes these
(`policy.rs:1916-1920`); `UnservicedLedger` records and always returns `None`. ⇒ **The fix shape
is to ADD A SERVING ARM, not to remove a refusal** — the two need different fixes and this is the
second.

★ Note the asymmetry the brief asked about: `0x2080200a` **is** on the capability allowlist
(`capability.rs:752`) and `0x20810108` is admitted by the `BinApiRule`. **Admitted and served are
different gates**; clearing the first raises no refusal and falls silently to the ledger.

### The guest driver SEES the `0x56` — and tolerates it

`binapiControl_IMPL` returns the physical-RM status verbatim
(`ogkm-580.159.04/src/nvidia/src/kernel/rmapi/binary_api.c:113-126`), as does
`kperfBoostSet_IMPL`. Measured on the wire in
`traces/guest_boots/run_w217_2f616e2_grpush_probe.log:525,527`: `rc=0` with `status=0x00000056` —
the syscall succeeds and libcuda reads the error.

### ★★★★★ THE MEASUREMENT — the injection ladder run PAST cuInit, for the first time

The committed matrix (`cuinit_fault_injection_matrix.txt`) says in its own header
*"`NVPROBE_STOP=1` (cuInit only)"*. ⇒ *"`0x20810108` is not load-bearing"* was a statement about
**cuInit**, and our wall is in **cuCtxCreate**. `cuinit_probe.c` has always had cuCtxCreate as
stage 4; only the driver script stopped early — **the instrument was truncated, exactly like the
recorder in Half A.** New driver: `scripts/rpctrace/inject_matrix_ctxcreate.sh`.

Real GA106, real `libcuda`, no QEMU (`traces/real_ga106/ctxcreate_fault_injection_matrix.txt`):

| row | `NVFAULT_CTRL` | INJECT_FIRED | `cuInit(0)` | `cuCtxCreate` |
|---|---|---|---|---|
| baseline | — | 0 | 0 | **0** |
| f_ctrl_20810108 | `0x20810108` | **1** | 0 | **0** |
| f_ctrl_2080200a | `0x2080200a` | **1** | 0 | **0** |
| f_ctrl_20800a9a | `0x20800a9a` | **0** | 0 | 0 |
| f_ctrl_all_three | all three | **2** | 0 | **0** |

⇒ **Forcing either control to `0x56` on healthy hardware leaves `cuCtxCreate` succeeding.** Both
status divergences are **inert for our wall**. The pre-registered arm "the two `0x56` are inert"
**FIRES**.

⊘⊘ **AND THE ROW THAT DID NOT FIRE IS THE ONE THAT MATTERS MOST.** `0x20800a9a` shows
`INJECT_FIRED=0` ⇒ **UNMEASURED, not inert.** libcuda never issues that id from userspace — it is
minted by the guest's *kernel* RM, so a userspace `LD_PRELOAD` interposer structurally cannot see
it. Without the `INJECT_FIRED` gate this row would have read as a fourth green "not load-bearing"
result. `[[a-census-zero-needs-a-known-positive]]`

### ⇒ SHOULD WE SERVE THEM? **Not on this evidence, and the brief's caution is upheld.**

The brief warned against serving a control merely because it diverges (`0x20801702` at w210 turned
a bounded failure into an unbounded hang). My measurement says these two **cannot** move the wall:
refusing them on hardware that works changes nothing.

⇒ Merging `origin/status-divergence` is a **conformance** change (it makes our status plane match
hardware's "non-OK exactly once in 613 records"), **not a fix**. It should be judged on
conformance grounds and pre-registered as changing **nothing** about the freeze. ⊘ I did **not**
merge it — that is a separate decision with its own risk, and this rung's job was to determine
whether it was warranted as a fix. It is not.

⊘ **Two structural limits of the injector, both stated in the script:** it can only turn `NV_OK`
into an error — it **cannot** produce a served-but-wrong answer, which is what our port is
actually suspected of; and it runs on a **healthy** stack, so a negative means "this refusal alone
does not break a working system", not "this id is cleared in combination with our other 428
divergences."

---

## THE FREEZE — no tie established, and I did not manufacture one

The brief's candidate (the `FAULT_PDE` fault kills the channel, so completions stop) remains **a
hypothesis with a mechanism and no evidence.** Nothing here tests it, and I did not build on it.

What w275 *does* contribute: the two `0x56` are now excluded as upstream causes, on hardware. That
removes a candidate rather than naming one.

★ The fault reproduces for the **fifth** consecutive boot, identical in engine/client/access/type
and channel, at this process's own ASLR slot:

```
Xid 31 ... channel 0x00000009 ... ENGINE GRAPHICS HUBCLIENT_FE
faulted @ 0x7ec5_10e00000 ... FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

⚠ Graded by **identity**, not count — and note engine and channel are **one** measurement reported
twice (`kchannelGetDebugTag` returns `(runlistId<<24)|ChID`), so they are not two facts.

---

## `CUP2_RC` — ⊘ NOT MEASURED THIS BOOT, and that is not a value

Anchored `grep '^CUP2_RC='` on `run_w275_pin_probe.log` returns **nothing**, because this boot ran
**`nvd_prog`** under the nvdiff hook, not `cup2`. ⊘ **An absent `CUP2_RC` is an absence of the
experiment, not a result** — it must not be read as "unchanged" or as "moved". The workload did
reach and hang at `cuCtxCreate` (stdout ends at `totalMem=11959 MiB`, exactly as `cup2` does),
which is the same wall by a different program — consistent with w274b's finding that the fault is
workload-independent.

⚠ The unanchored form returns `[]` here too, so this boot happens not to expose the
`GCC_CUP2_RC=0` decoy — that trap is still live for boots that run `cup2`.

---

## DEVICE COMPARABILITY — the port is the constant, asserted not assumed

| counter | w275_pin | w271_pin | w274_pin |
|---|---|---|---|
| `DOORBELL-XLATE` | **88** | 88 | 88 |
| `OPERAND-PIN` | **156** | **156** | 223 |
| doorbells served / forwarded | **196 / 12** | 201 / 12 | — |

All six ARM ASSERTIONS PASS (`FB-JOIN=shared`, `GUEST-RING=ring`, `GUEST-PUSHBUF=pin`,
`GUEST-SEMA=pin`, `GR-ROUTE=passthrough`, `GUEST-OPERAND=pin`). `STAMP=HEAD=55c5d16`.

⊘⊘ **AND A CORRECTION I OWE, IN THE GRADER, FIXED IN PLACE.** Both runners printed
`OPERAND-PIN … (w271_pin: 88)`. **88 is the DOORBELL-XLATE value one row up** — carried forward
from w273's summary instead of read from the log. `[measured, read from the artefacts]`:

```
traces/boots/w271/run_w271_pin_qemu.log      OPERAND-PIN=156  DOORBELL-XLATE=88
traces/boots/w274/run_w274_pin_qemu.log.gz   OPERAND-PIN=223  DOORBELL-XLATE=88
traces/boots/w271/run_w271_off_qemu.log      OPERAND-PIN=0    DOORBELL-XLATE=17
```

The off-arm row shows the counter is real (0 when disarmed), so only the baseline was broken. Both
numbers are now pinned separately in `w274b_run.sh` and `w275_run.sh`.

---

## PRE-REGISTERED ARMS — how they fell

| arm | outcome |
|---|---|
| the untruncated record shows a real payload difference | ⊘ **DID NOT FIRE** |
| it shows none ⇒ the divergence is ordering, not content | ★ **FIRED** — recovered bytes identical; the only content diff is the GPU UUID |
| the two `0x56` are inert | ★ **FIRED** — on real hardware, `cuCtxCreate` = 0 under both injections |
| serving them changes the freeze | ⊘ **NOT TESTED** — and now unmotivated: they cannot move a wall they do not gate |
| `CUP2_RC` moves | ⊘ **NOT MEASURED** — this boot ran `nvd_prog`, not `cup2` |

★ Six of the last eight rungs had their least-weighted arm fire. Here the *least*-weighted arm
("no payload difference at all") is the one that fired.

---

## ⊘⊘ COVERAGE — per the owner's rule, an absence claim without coverage is not a finding

**Covered.** One stage (`ce`, the `cup2` shape, zero kernel launches); one guest run and two host
runs; `/dev/nvidiactl` + `/dev/nvidia0` + `/dev/nvidia-uvm*`; every ioctl's header **and**
out-of-line parameter bytes on **both sides** of the call, now **untruncated** (0 truncated
records, max struct 9264 < 65536); return value and errno; `mmap`/`munmap` of nvidia fds. Guest
436 records, host 578. Plus a native 5-row injection matrix over 3 control ids, with firing
asserted per row.

**NOT covered, and none of it is a small omission:**

- ★★★ **Everything that is not an ioctl** — BAR/MMIO, the doorbell, USERD `GP_PUT`/`GP_GET`, the
  pushbuffer, the GPU's DMA writes into guest RAM, interrupt delivery, and **the completion
  plane**. *The wall lives there.* This rung's headline (`0x20801702` ×76) is a **shadow** of a
  data-plane fact, not the fact.
- ★★★ **The freeze itself is untouched.** No sampling of the completion page this boot; the
  "frozen, not lagging" result is w274's and is not re-measured here.
- **The guest arm has no noise floor of its own** — one run. Only the host's was measured (zero
  over 578×2). A guest-side divergence that is genuinely run-to-run noise would be indistinguishable
  from a finding.
- **The injector cannot produce a served-but-wrong answer**, only `NV_OK → error`; and it runs on
  healthy hardware, so it bounds each id **in isolation**, never in combination.
- **`0x20800a9a` is UNMEASURED** by that instrument, structurally — the id never crosses the
  userspace boundary it interposes.
- **One workload, one chip (GA106), one driver (`580.159.04`), one guest boot.** `nvd_prog`
  launches no kernel, so nothing about `cuLaunchKernel` is measured.
- **The 52 non-UUID value divergences are named but not explained** — `0xc36f0108` (16) and
  `0x00800292` (12) in particular are un-analysed and are now the largest un-attributed block.

---

## Artefacts

| what | where |
|---|---|
| boot logs (serial, dmesg, hostdmesg, probe, qemu.gz, run log) | `traces/boots/w275/` |
| host + guest captures, README with the full analysis | `nvidia-gpu-passthrough@4bf4dd0:traces/nvdiff_w275/` |
| cuCtxCreate injection matrix | `traces/real_ga106/ctxcreate_fault_injection_matrix.txt` |
| the injection driver | `scripts/rpctrace/inject_matrix_ctxcreate.sh` |
| recorder fix + `"tbl"` exemption | `nvidia-gpu-passthrough@875c50c` |

**Every number in this document was read from an artefact opened in this session.**
