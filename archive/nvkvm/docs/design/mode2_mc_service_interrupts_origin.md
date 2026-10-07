# Who issues `MC_SERVICE_INTERRUPTS` (`0x20801702`), and what puts the driver there

    STATUS: LIVE (2026-08-11). ANSWERED from committed captures; no boot was required.
    Every number below is re-derivable offline from this repo with no GPU.
    Subjects: traces/host_reference_ga106/  (real GA106, closed 580.173.02, five-GPU rig)
              traces/guest_mode2_vh2_s2/    (kayfabe 28cf456 on vh2, guest open 580.159.04)
    Differ:   tests/mode2/nvdiff/nvdiff.py  — nvd_selftest.sh PASS (479 / 5) before every diff here.

## ⊘ THREE CORRECTIONS FIRST — the framing of the question was wrong in three places

**⊘ 1. The caller is NOT UVM, and the two `uvm_gpu.c` routes are not on this plane at all.**
The 175 calls are **userspace `ioctl()`s** on `/dev/nvidiactl`, recorded by an `LD_PRELOAD`
shim inside the CUDA process. UVM's kernel routes reach RM through
`nvUvmInterfaceServiceDeviceInterruptsRM` → `pRmApi->Control(...)`
(`ogkm-580.159.04/src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:7840`) — an **in-kernel** call that
issues **no syscall** and therefore **cannot appear in this instrument at all**. Whether those
two routes are open or closed is independent of what was measured.

Three facts pin the caller to userspace, all from the capture itself:
- the poll's `hClient=0xc1d0001b` is allocated at `ctx_r1[7]` (`NV01_ROOT_CLIENT`, class `0x41`)
  and its `hObject=0x5c000003` at `ctx_r1[35]` (class `0x2080`, the subdevice) — **the CUDA
  process's own handles, created earlier in the same trace**. A UVM-originated control carries
  UVM's session client, never the process's.
- same `tid` as record 0, i.e. the workload's main thread, inside `cuCtxCreate`.
- ★ **the decisive one — zero kernel-originated surplus.** The device-side log for this boot
  reports `control 0x20801702 result 0x00000000 x350`, and the boot ran four workload runs
  (`dev` ×2 = 0 polls, `ctx` ×2 = 175 each) ⇒ **350 emitted from userspace, 350 seen at the
  emulated GSP.** `subdeviceCtrlCmdMcServiceInterrupts_IMPL` RPCs to GSP for *any* caller on a
  GSP client (`gpu/intr/intr.c:189`), so a UVM-originated call would have shown as surplus.
  There is none.

**⊘ 2. `175` (and `165`) are artefacts of the measuring window, not facts about the system.**
`scripts/mode2_diag/nvdiff_run_guest.sh:102` sets `NVD_TIMEOUT=180`; the manifest records
`prog rc=124`; the device log's two refusal batches are **181.075 s** apart. `nvd_prog.c`
`fflush()`es after every line and printed no `CTX OK` and no `FAIL cuCtxCreate`, so
`cuCtxCreate` **never returned** — the process was killed mid-loop. The loop is **unbounded**;
`175` is `(180 s − startup) × cadence`. ⊘ Do not build on `165` vs `175`; they are the same
unbounded loop counted against two different clocks. *(The shim has no record cap — only
`NVDIFF_MAXBUF` on parameter bytes — so this is a timeout bound, not an instrument bound.)*

**⊘ 3. The polls are not a divergence to be explained on the control plane.** See below: the
control plane is clean through the wall. Nothing we *answer* puts the driver there.

## The wall, by kind (⊘ never by index — first by index is `CARD_INFO`, environmental)

The two streams run in lockstep and the alignment is exact. Last **agreeing** record:

    A[349] / B[314]   nvidia-uvm : UVM_MAP_EXTERNAL_ALLOCATION    rc=0 on both

Its only divergences are the 16-byte GPU **UUID** at `hdr@0x18` and handle low bytes — identity,
not semantics. Then the streams part in a single step:

    A[350]            nvidiactl : RM_ALLOC cls=0x0040   (NV01_MEMORY_LOCAL_USER — video memory)
    B[315…489]        nvidiactl : RM_CONTROL 0x20801702 × 175, params 0xffffffff, status NV_OK

`0xffffffff` is `NV2080_CTRL_MC_ENGINE_ID_ALL` (`ctrl2080mc.h:179`).

**The control plane is clean.** Over the whole aligned program the differ's **STATUS class fires
0 times** — for every record both sides return the same status, and hardware's single non-OK
(`0x2080012f` in `cuInit`) is matched exactly. The residual `UNEXPLAINED` mass ranks as: 84
`CARD_INFO` (five-GPU reference rig), then work-submit tokens, handles, and the GPU UUID. **No
reply value we produce differs from hardware in a way that could route the driver here.**

⇒ The divergence is **not a wrong answer. It is an absent event.**

## What the driver is actually doing there

`subdeviceCtrlCmdMcServiceInterrupts_IMPL` (`gpu/intr/intr.c:189`) carries NVIDIA's own comment:

> Force kernel-RM to service interrupts from GSP-RM. This will allow kernel-RM to **write
> notifiers** and send an ack back to GSP.

That is the call a client makes **while waiting for a notifier**. Real hardware calls it
**zero times in all 12 host captures** because on real hardware the notifier arrives.

The device-side census for this same boot names what never completed, in its own words:

    doorbells: 424 arrived, 408 served, 16 REFUSED by name
      by engine: GrCompute=16 GrGraphics=0 Ce=408
      of the served: 408 local (CPU CE, end witnessed), 0 forwarded (host channel rung)
    isolates: 5 materialized, 5 live, 5 refusing (5 no-plane)
    DOORBELL-REFUSED #1..#8  tokens 0x07..0x0e  [Route::NotACopyEngineChannel]
    GR-PUSHBUFFER token=0x00000007 engine=GrCompute ring=0x200200000 methods=86 bytes=864

Tokens `0x07..0x0e` are the eight channels created at `ctx_r1[132…214]` — **before** the poll
onset at `[315]` — and their `GET_WORK_SUBMIT_TOKEN` replies in the capture are exactly
`0x07…0x0e`. So: the guest submitted **real GR compute work** (86 methods), this build **has no
forwarding plane** (`5 refusing (no-plane)`, `0 forwarded`), the doorbells were refused by
routing, nothing executed, no completion was written — and `cuCtxCreate` then sat in a
notifier-wait poll until the harness killed it.

⚠ **Honest scope.** (a) the control plane is clean, (b) GR execution provably never happened,
(c) libcuda is in a notifier-wait poll — all measured. The *link* from (b) to (c) is an
inference about closed-source libcuda's intent and is **not proven here**; the oracle cannot see
completions by construction.

## The falsifier, and it is one record wide

The oracle states the expected next record exactly. If GR work is made to complete, the guest's
ioctl immediately after `UVM_MAP_EXTERNAL_ALLOCATION` must become **`RM_ALLOC cls=0x0040`**, and
the progress fraction must move off **221/479 = 46.1 %** — a number that has now been identical
across two boots and two source revisions, so any movement at all is signal. If the polls persist
*after* GR completions are delivered, this account is refuted and the wait is on something else.

⇒ **The polls are a symptom, and they are adjacent to the wall rather than the wall itself.** The
instrument that can see the wall is not this oracle — it is the device-side doorbell/routing
census quoted above, which is **already armed and already answering**.
