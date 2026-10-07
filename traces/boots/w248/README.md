# w248 — property 3 MEASURED: a guest-reachable GPU fault is **CONTAINED**

⊘ **No guest ran and nothing was forwarded.** `CE-SUBMIT` does not apply — this rung ran
`scripts/bench/gpu_fault_containment.sh`, an instrument that already existed and had never been
asked this question. **No code was written.**

Host `vh`, RTX 3060 (GA106), driver 580.159.04. `SCRIPT_RC=0`.
**All five predictions confirmed** — `predictions_recorded_before_the_run.md`, committed beside
the log and written before the run.

## The result

| arm | result |
|---|---|
| **A** baseline victim, idle GPU | `[victim] OK bad=0` rc=0 |
| **B** attacker faults alone | `sync rc=700 (CUDA_ERROR_ILLEGAL_ADDRESS)`; `context reusable? rc=700 NO (sticky)`; **Xid 4 → 5** |
| **B2** fresh victim after the fault | rc=0 (⊘ weak arm — fresh context, by construction) |
| ★★★ **C** victim holds a **LIVE** context across the fault | `[loop] DONE iters=2675519 ok=2675519 wrong=0 errors=0` → **victim exit=0** |
| **D** aftermath | Xid total 6; brand-new victim rc=0; **no** `fell off the bus`, **no** reboot-required |

★★★ **2 675 519 verified iterations of a bystander context, spanning the attacker's MMU fault,
zero errors and zero wrong bytes.** The fault is scoped to the offending context.

★★ **The Xid is exactly property 3's shape** — the graphics engine, and a *page-directory* fault,
so the unmapped VA aliased nothing:

```text
NVRM: Xid (PCI:0000:00:07): 31, name=gpu_wedge_probe, channel 0x00000008,
  MMU Fault: ENGINE GRAPHICS GPC1 GPCCLIENT_T1_0 faulted @ 0x7000_00000000.
  Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

## ⚠ Scope — recorded before the result, so it could not be fitted to it

The attacker is a **host CUDA process faulting in its own context**, not guest methods on our
isolate's GR channel; the script says so itself.

- **Established**: the blast radius of a GR MMU fault + Xid 31 on this GPU and driver is the
  **faulting context**. `guest_blast_radius.md` §7's escalations — whole-runlist preempt,
  node-level reboot latch, GSP death — **did not occur**.
- **Not established**: that a fault raised from *our* GR channel, in a VAS *we* built, by *guest*
  methods has the same radius. That needs GR execution, which **property 2** blocks.

★ **Corroboration we did not go looking for**: this host's `dmesg` carries **four earlier Xid 31
MMU faults raised by kayfabe itself** — `kayfabe-rm-ladd` ×3, `a_guests_ring_m` ×1, all
`ENGINE CE0 … FAULT_PDE ACCESS_TYPE_VIRT_READ` — and the bench has booted normally ever since.
**We have already faulted this GPU four times and it survived every one.**

## ★★★★★ The sixth instance — the host's own dmesg has been diagnosing us by name

Our census prints `REFUSED Rm(Other(64))` — **12 in `w247` alone**. The host driver, on the same
box at the same instant, prints the cause:

```text
NVRM: kfifoRunlistSetId_GM107: Channel has already been assigned a runlist incompatible
  with this engine (requested: 0x2 current: 0x0).
NVRM: chandesConstruct_IMPL: Invalid object allocation request on channel 0x0000000c
```

**241 of each, paired 1:1**, across this campaign's boots. And the number decodes:
`NV_STATUS_CODE(NV_ERR_INVALID_STATE, 0x00000040, …)` — `64 == 0x40`
(`ogkm-580.159.04: kernel-open/common/inc/nvstatuscodes.h:93`).

⇒ **We print a number; the driver prints the sentence; no rung has ever read it.** The first
instance tonight where the unread instrument is **not ours**.
⚠ `[CORRELATION]` — paired by timestamp and count, not joined on a channel id.
⊘ `boot_capture.sh` captures the **guest's** dmesg only. **Capture the host's too.**

Full record: `docs/design/execution_plane_increments.md` §16.100.
