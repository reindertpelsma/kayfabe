# V3 — ACPI S3 suspend/resume of a kf3 guest

**STATUS: DESIGN + CODE, 2026-10-14 — GPU-free tested; hardware result in §6 (pending at the time of
writing).** Branch `claude/s3-resume-20261014`. Evidence of the failure: `traces/linux_s3_resume_20261014/`.
Measured = a run or a GPU-free test named here; inferred = read from source, not executed.

## 1. The failure, measured (kf3 `c287af4f`, Linux desktop guest, ogkm 580.159.04)

The guest suspends cleanly (`UNLOADING_GUEST_DRIVER bInPMTransition=true newLevel=3`, GSP
`Running -> Suspending -> Halted`). QEMU's `system_wakeup` resets every device, and kf3 took that as
a full device reset (`device reset #2 … GSP Halted -> Cold`, 6 VA spaces retired, the chain rebuilt).
The guest's `pci_pm_resume` then runs `kgspBootstrap_TU102(KGSP_BOOT_MODE_SR_RESUME)`, prints
`Failed to boot GSP.` and `GSP boot failed at resume (bootMode 0x1): 0x55`, then Xid 119 on
`GSP_RM_ALLOC seq 1719` (its command sequence continues from fn 47's 1718).

## 2. What the guest does on resume (read: `ogkm-580`, tag 580.159.04)

- `gpuPowerManagementResume` (`gpu_suspend.c:150-290`): `kgspWaitForGfwBootOk`, then
  `kgspPrepareForBootstrap(SR_RESUME)` (no FWSEC prepared), then `kgspBootstrap(SR_RESUME)`.
- `kgspBootstrap_TU102` (`kernel_gsp_tu102.c:493-578`): the scrubber if needed; **no** FWSEC, **no**
  `kflcnResetIntoRiscv`, **no** boot-args mailbox pair (all `bootMode == NORMAL` only); Booter Load on
  SEC2 with the suspend/resume metadata address in SEC2 `MAILBOX0/1`; `NV_PFALCON_FALCON_OS`; then it
  requires `kflcnIsRiscvActive || _kgspIsProcessorSuspended`, else `NOT_READY` (0x55); then, without
  `GspStatusQueueInit` (*"for resume or GC6 exit, GSP-RM will restore queue state"*), it waits for
  `GSP_INIT_DONE` on the queue it kept (`kgspWaitForRmInitDone`, `kernel_gsp.c:5214-5240`).
- The suspend side (`kgspUnloadRm`, `kgspTeardown_TU102`): fn 47, the suspend poll, FWSEC-SB on the
  GSP falcon (our E2: `Suspending -> Halted`), then Booter Unload with the SR metadata — skipped by the
  guest because WPR2 already reads down (`kernel_gsp_booter_tu102.c:148-153`).
- On silicon GSP-RM's state (its heap: every RM object, the queue state) crosses D3cold inside
  Booter's suspend/resume data; CPU-RM saves video memory itself (FBSR).
- QEMU (10.2.4) resets every device on the wake with `RESET_TYPE_WAKEUP`
  (`hw/i386/pc.c:1723-1727` `pc_machine_wakeup`; both `pc` and `q35`), distinct from
  `RESET_TYPE_COLD`.

## 3. Hypotheses and their falsifiers (stated before the run)

- **H1 — the FSM has no resume.** After the wake the guest's writes are SEC2 Booter Load and
  `FALCON_OS` with the phase `Cold`; no GSP STARTCPU and no GSP boot-args mailbox pair follow, so
  nothing brings the phase up and `RISCV_CPUCTL` reads inactive. *Falsifier:* the per-life write log
  (§5) shows a GSP falcon STARTCPU or a GSP `MAILBOX0/1` pair between the wake and the guest's
  `Failed to boot GSP`, or (after the fix) the FSM reaches `Booted` and the guest still prints
  `Failed to boot GSP.`
- **H2 — the wake's full reset destroys what GSP-RM keeps.** Even with the FSM resumed, a full reset
  frees the guest's clients (host twins), retires its address spaces and rebuilds the chain, so the
  resumed guest's RPCs and channels refer to objects that no longer exist. *Falsifier:* after the fix
  (which keeps them), the resumed guest's RPCs are refused for unknown handles, or its channels make
  no progress (glxgears 0 frames after the wake). ⊘ Not ablated (an FSM-only build was not run): H2's
  necessity is inferred from the reset's own stages (`V3_DEVICE_RESET.md` §3).
- **H3 — QEMU names the wake.** kf3's hold phase sees `RESET_TYPE_WAKEUP` for `system_wakeup`.
  *Falsifier:* the fixed build logs a plain `device reset #N: vCPUs out …` (not `(S3 WAKE)`) at the
  wake although the guest suspended with `bInPMTransition=true`.

## 4. The design (hardwired, no flag)

**FSM (`kf-gsp`, `boot.rs`).**
- fn 47 decoded as a PM transition (not GC6) is recorded (`GspFsm::note_pm_suspend`, from the device,
  which already decodes the body).
- Entering `Halted` from `Suspending` keeps the binding **inert** (`suspended`): no read goes through
  it (a doorbell while `Halted` is still E8, zero guest-RAM reads).
- **E14** `wake_reset`: only from `Halted` with a PM-saved binding — every register back to power-on
  (`Cold`), and the binding, both cursors, both sequence numbers and the region identity kept.
  Anything else returns `None` and the caller does the total E11.
- **E15** resume: a new `BootStep::FirmwareResume`, emitted by the falcon/booter sequence for a SEC2
  Booter Load (not Unload) while `Cold`/`Halted` — the SR_RESUME shape. With a saved PM life the FSM
  goes `Booted` (WPR2 up, RISC-V active), restores the binding **without publishing** (no header
  write, no re-link, no sequence reset), posts `GSP_INIT_DONE` at the status stream's next sequence
  number, and reaches `Running` once the guest's read pointer moves off its value at the resume
  (`resume_mark`; a resume never re-links, so "off zero" is wrong there). Without one it does nothing
  (Ada's scrubber on a cold device takes that path).
- A FWSEC start (E1), a publish (E6) or E11 discards a saved life: the guest booted instead.

**Device reset (`kf-qemu`).** ABI 29: `kf3_device_reset(h, from_vcpu, wake)`, `wake` = the reset
type is `RESET_TYPE_WAKEUP`. A run covering only wake requests asks the device to keep the life; the
device keeps it only if `GspFsm::wake_reset` succeeds (`reset::Life::Kept`). A kept run: the
privileged ring discarded, the interrupt tree (and FSP EMEM) reset, the GSP registers back to
power-on; **Release and Memory are skipped** (host twins, tokens, address spaces, BAR views, the
chain's objects, the display all kept); Finish keeps the tokens (the life's doorbells), restores the
register shadow to the realize image and republishes. Any cold request in the run, or no saved PM
life, is the full reset of `V3_DEVICE_RESET.md`. A kept run marks the device touched, so a cold reset
straight after it is not "pristine" and ends the kept life.

Hostile guest: nothing the guest controls reaches the host; the resume reads one guest word through
a geometry validated at the original bind; a guest can only make kf3 keep its OWN life across a
wake, which a cold reset still ends.

## 4b. The guest's FBSR save, served (added after run b1)

`[measured b1, kf3 44e9b092]` with §4 alone the GSP resume worked (`GSP-RESUME … stat_seq=1740
cmd_seq=1741`, `Cold -> Booted -> Running`, the guest past `kgspBootstrap`), and the next wall was the
suspend's video-memory save, which had been failing all along: `_fbsrInitGsp` registers GSP-RM's
suspend buffer with `ALLOC_MEMORY` (fn 4, a PTE list longer than one message, so a head plus a
`CONTINUATION_RECORD`) and kf3 refused both fragments **separately** (`REFUSED fn4=0x56`, `REFUSED
fn71=0x56`; the guest: `Unexpected RPC event … CONTINUATION_RECORD`); `fbsrBegin` failed after creating
its CeUtils, every later FBSR type asserted `pCeUtils == NULL` (`mem_mgr.c:4122`), no save was valid,
and at resume `memmgrRestorePowerMgmtState` (*"Calling Resume on an active GPU or the previous Suspend
call might have failed"*) failed `gpuStateLoad` — `nvidia-smi` and glxgears dead after the wake, 0 Xid.
Also refused during the suspend: `INTERNAL_FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING` (`0x20800ac3`),
whose resume-side refusal `kfifoStateLoad` returns (`kernel_fifo_init.c:70-93`).

- `kf-gsp` (`large.rs`, `boot.rs`): a large `ALLOC_MEMORY` is joined like a large control (its length
  from the version's generated MemoryList contract, `kf_abi::memory_list::Cell::declared_len`), and,
  being `bBidirectional = NV_FALSE` (`_issuePteDescRpc`), answered with ONE reply at the head carrying
  only the status (`rpc.c:2133-2210`).
- `kf-rm` (`pmsr.rs`): the SYSTEM MemoryList registration, its FREE, `INTERNAL_FBSR_INIT`
  (`0x20800ac2`) and the scheduling toggle are answered `NV_OK` with no host action — the reasons
  and what they do not claim are in the module's docs.
- *Falsifier (H4):* with these answered, the guest still prints the `pCeUtils == NULL` asserts or
  *"Calling Resume on an active GPU"*, or `gpuStateLoad` fails at resume.

## 5. Instrumentation

The first 512 privileged writes of every logged device life are logged (`kf3: w#N bar0 @off = v
phase=…`), restarted at each logged reset — the resume's SEC2/GSP traffic is in `qemu.log` after the
`(S3 WAKE)` line. `kayfabe: GSP-RESUME …` names the restored cursors and sequences.

## 6. Results

Pending (filled in by the run that measures it).
