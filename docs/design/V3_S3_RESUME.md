# V3 — ACPI S3 suspend/resume of a kf3 guest

**STATUS: LIVE, 2026-10-14 — MEASURED on the trusted host (RTX 4070, kf3 `559db30c`): the Linux desktop
guest (ogkm 580.159.04, stock module) survives three S3 suspend/resume cycles in one boot (§6).** Branch
`claude/s3-resume-20261014`. Evidence: `traces/linux_s3_resume_20261014/` (the failure) and its `fixed/`
(the passing runs). Windows 11: run 2026-10-14, NOT yet surviving (§7a, correction above §7). **UPDATE 2026-11-23 (§7b, above
§7a): the Windows post-wake hang is root-caused (QEMU's wake reset wipes the Hyper-V TSC page while the TSC restarts) and
fixed by a third QEMU hunk; 3/3 cycles with it on one box; each wake is still followed by a guest TDR recovery.** **UPDATE 2026-11-23 (§7c, above §7b):
that TDR has two causes; the first (every RAM lease revoked by the wake's flat-view re-split) is fixed, the second (kf3's display
engine stops window channel 1 after the wake) is open, so Windows still takes a TDR after every wake.** Measured = a run or a GPU-free test named here; inferred =
read from source, not executed.

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

## 4c. Walls after b2 (each found by the run before it, batched where the source showed them)

- `[measured b2, 91afc335]` the FBSR save now valid (no `pCeUtils` asserts, no *"Calling Resume on an
  active GPU"* — H4's falsifier not hit). Next: at resume `kbusRestoreBAR1ResizeSize_WAR_BUG_3249028_GA100`
  asserted `bar1ResizeSizeIndex >= NV_XVE_RESIZE_BAR1_CTRL_BAR_SIZE_MIN` and failed `gpuStateLoad`
  (`kern_bus_ga100.c:1501`, `kern_bus_gm107.c:551`): the BAR0 XVE mirror served 0 for
  `NV_XVE_RESIZE_BAR1_CTRL`, which RM caches at load. ⇒ `kf_chip::bar0` serves it on GA100…AD107 as
  this device's own BAR1 size in the Resizable-BAR encoding (`log2(bytes) - 20`, field `12:8`, offsets and
  bounds from the hwref table — four rows added to `hwref-580.159.04.tsv` by `tools/derive_hwref.sh`).
  Hopper/Blackwell read the config space's Resizable-BAR capability, absent here, which clears the WAR.
- `[measured b3/b5, 89362cbe]` `gpuStateLoad` completed (the remaining asserts are the boot's own,
  identical before the suspend); glxgears rendered 302 frames after the wake, but `nvidia-smi` hung.
  `[measured b6]` strace: it spun on `UVM_INITIALIZE` → `EAGAIN`. `[read]` cause: kf3 refused
  `NV2080_CTRL_CMD_FIFO_DISABLE_USERMODE_CHANNELS` (`0x20801117`, ROUTE_TO_PHYSICAL), so the procfs
  suspend's `nv_preempt_user_channels` failed (the WARNs at `nv.c:4472/4759`), and that error path calls
  `nv_uvm_resume()` without a prior `nv_uvm_suspend()` (`nv.c:4759-4810`) — `uvm_resume` releases
  `pm.lock` it never took (`uvm_global.c:360-400`), after which every UVM ioctl's `down_read_trylock`
  fails (`uvm.c:1058-1060`). ⇒ served in the channel plane (`ChanStatement::DisableUsermode`): the host
  disables this VM's user twins (host `DISABLE_CHANNELS`, unprivileged, our own channels, an act) and
  re-enables exactly those at resume.

## 5b. What the guest sees across the wake (from the per-life write log, run b1)

H1 holds: after `device reset #2 (S3 WAKE)` the guest's writes are SEC2 Booter Load and `FALCON_OS`
with the phase `Cold`; no GSP falcon STARTCPU and no boot-args mailbox pair occur, and E15 fires on the
Booter Load (`GSP-RESUME … no publish`). H3 holds: every wake reset arrived as `RESET_TYPE_WAKEUP`
(`(S3 WAKE)` lines, b1-b8). H2 was not ablated (inferred, see §3).

## 6. Results (kf3 `559db30c`, trusted host, RTX 4070, host driver 595.91.07)

- `desktop_s3.sh 559db30c b7` (1 cycle) PASS; `S3_CYCLES=3 desktop_s3.sh 559db30c b8` **3/3 PASS** in one
  boot: each wake `(S3 WAKE) … KEPT`, `GSP-RESUME` with continuing sequence numbers (stat 1683 → 2644 → 3644),
  ssh back in ~3 s, `nvidia-smi` names the RTX 4070, glxgears 301/302/301 frames in 5 s, 0 Xid, 0 kernel
  WARNINGs, no GSP refusal after the first fn 47. `traces/linux_s3_resume_20261014/fixed/`.
- GPU-free: `cargo test` over every `kf-*` crate 3007 passed / 0 failed; `v3_gates.sh` 9/9, gate 10 PASS,
  12/12 user births.
- Fast suite (`KF_DEVICE=kf3`, 180 s budget) **30/30**; `desktop_smoke.sh 559db30c`: NVIDIA renderer,
  glxgears ~60 FPS, 0 flip timeouts, 0 Xid (guest and host), 18 kf3 display STALL lines (9 at
  `41acc6ac`, same family, no guest-visible effect); `planes_oracle.sh 559db30c`: rgb/nv12/p010/yuyv
  PASS (0 mismatched), 0 flip timeouts in every kfplanes test, host Xid delta 0 — the remaining
  verdicts (vblank-wait/-event FAIL −95, scaled overlays SKIP, the two cycles SLOW) are the baseline of
  `V3_DISPLAY_PLANES_ORACLE.md` §tables. `traces/linux_s3_resume_20261014/fixed/validation-559db30c/`
  (its `summary.log` also holds the first fast attempt, NOTRUN: the raw client was built into another
  target dir — a harness slip, rerun).

## 7c. UPDATE 2026-11-23: the post-wake TDR, first cause fixed, second cause open (corrects §7b's "Not fixed" below)

**STATUS: RESEARCH, 2026-11-23. MEASURED on Vast instance 62141661 (RTX 4090, nested KVM, L1 clocksource tsc), control kf3
`90a658c3` and fix kf3 `0c16cb72`, q35, arm B, Windows 11 LTSC with 580.88; evidence `traces/win11_s3_tdr_20261123/README.md`
(hypotheses committed before the runs).**
- §7b's lead was not the cause: "code 76" is the RPC function `GSP_RM_CONTROL`, and the controls left UNSERVICED after the
  wake are the ones every Windows boot leaves unserviced (read; PREREGISTRATION.md of that directory).
- [measured T0, control] six user-ring relays born before the wake refused `USER_RING_PUT_READ: Topology` at their first
  access after it, and the TDR came 24 s after the wake. Cause [read + measured]: `RamMap` moved its generation on every
  guest-RAM section QEMU added or removed; a q35 wake resets the MCH, the SMRAM/TSEG windows toggle and the flat view
  re-splits the low-RAM section (same file bytes at the same addresses); every lease that captured the generation (sysmem
  USERD, user-ring source snapshots, MemoryList registrations) was dead for good. **Fix (F2, hardwired):** the generation
  counts changes of the guest-RAM translation (guest-physical address to backend file offset): an fd-backed section that
  maps a byte or an address differently from any translation registered before revokes; a re-split of the same linear map,
  and a removal (a removed range is refused at lookup), do not. kf3 logs each `RAM add`/`RAM del` with `translation kept`
  or `translation CHANGED`. Tests: `a_wake_resplit_of_the_same_ram_keeps_a_resolved_userd`,
  `a_wake_resplit_of_the_same_ram_keeps_the_birth_generation` (fail before the fix with `Err(Topology)`), and the conflict
  and bound tests in `mem.rs`. [measured T1, T1b with F2] no Topology refusal; every wake RAM change `translation kept`.
- **Still open:** [measured T1, T1b] with F2 the TDR still follows every wake, 68-70 s after it: about 12 s after the wake
  kf3's display engine halts window channel 1 (`PUT 0x1130 / GET 0xe20 outside the 4096-byte pushbuffer`, the guest
  declared 4 KiB), then the core channel parks waiting for that window's interlock. [inferred] the guest's window-channel
  PUT and the GET kf3 kept across the wake disagree. The release row `windows-sleep-wake` stays expected-FAIL.

## 7b. UPDATE 2026-11-23: the Windows post-wake hang, root cause and fix (corrects §7a below)

**STATUS: RESEARCH, 2026-11-23. MEASURED on Vast instance 62082978 (RTX 4090, nested KVM, L1 kernel 6.8.0-59), kf3 `1052e2f9`,
q35, Windows 11 LTSC with the 580.88 driver from `windows_gate.sh`; evidence `traces/win11_s3_wake_20261123/README.md`
(hypotheses committed before the runs).**
- [measured R1t] The 2026-10-14 storm reproduces on q35 only when the host gives KVM a master clock (L1 rebooted with
  `tsc=reliable clocksource=tsc`), i.e. when the guest's Hyper-V TSC page is valid. At the wake QEMU's cold CPU reset zeroes
  every Hyper-V MSR field of `CPUX86State` (they sit before `end_reset_fields`) and writes them to KVM, so
  `HV_X64_MSR_REFERENCE_TSC = 0`: KVM stops maintaining the page, which stayed byte-identical in guest RAM. The guest TSC went
  back by the guest's uptime (TSC offset -588,320,593,649 cycles = -261.48 s at 2.25 GHz). Windows did not re-enable the page
  (no write of 0x40000021, 0x40000000 or 0x40000001 after the wake), so its reference time read the stale page and went
  negative; the first post-wake one-shot was `count = -0.3979 s` with KVM's `time_now = 261.08 s`. KVM's `stimer_start`
  computes `100 * (count - time_now)` in u64, which wraps, so the hrtimer is in the past, `kvm_hv_process_stimers` never sees
  the (u64) deadline reached and re-arms forever: the guest waits in HLT for a timer that never fires.
- [measured R1] On a nested box whose L1 TSC is "unstable" (kvm-clock), the page is invalid (all zeros), Windows reads
  `TIME_REF_COUNT` (always KVM's) and the wake is clean with stock QEMU: the hang depends on the host. §7a's inference that
  the TSC is reset to 0 by the wake now holds as measured on a stable-TSC host; its inference that the common factor is
  i440FX + OVMF is withdrawn for arm B (q35 hangs the same way); arm A on q35 was clean once (R2).
- **Fix (hardwired, no flag):** a third QEMU source hunk, `pc_machine_wakeup` keeps each vCPU's Hyper-V MSR block
  (`msr_hv_hypercall` .. `msr_hv_tsc_emulation_status`) across the `RESET_TYPE_WAKEUP` reset: read from KVM, reset, put back,
  then written to KVM, so KVM republishes a valid page for the restarted TSC. Architectural CPU state is reset as before; a
  guest without `hv-*` flags carries zeros through it. One implementation, `scripts/release/build_qemu.py` (`WAKE_NEW`),
  applied by the release kit and by `scripts/bench/build_kf3.sh` (`build_qemu.py wake-hunk`). [measured R3] 3/3 clean cycles
  with it on the stable-TSC boot (page republished, sequence 5; post-wake deadlines 0.4-1.4 ms ahead). A stock QEMU, or a
  future loadable-module build, lacks it. Test: `scripts/ci/test_release_qemu.py` (`test_the_wake_hunk_*`,
  `test_build_kf3_applies_the_wake_hunk_before_it_builds`); it pins the hunk's shape and order, the hardware runs pin its effect.
- **Not fixed:** about 34-45 s after every wake (R1, R2, R3) the Windows driver logs event 153 "Resetting/Reset/Restarting TDR"
  and its RM unloads (`fn 47 ... newLevel=0`) and comes back on a new GSP life; desktop and `nvidia-smi` work afterwards. Lead:
  repeated `GSP rpc UNSERVICED { code: 76 }` between the wake and the unload (not analysed; ⊘ superseded by §7c: not the
  cause; the TDR has two causes, one fixed). The release row
  `windows-sleep-wake` stays expected-FAIL until a run without the TDR passes.

## 7a. Windows 11 run (2026-10-14, kf3 `0c5fedff`) - correction folded above §7

**STATUS: RESEARCH. Windows 11 does not survive S3 wake yet (0 clean cycles in 7 attempts); the failures are before or
beside the GSP.** Evidence and per-run table: `traces/win11_s3_20261014/README.md`.
- [measured] Windows' suspend IS served by this branch's code: fn 47 `bInPMTransition=true newLevel=3`,
  `Running -> Suspending -> Halted`, no `GSP REFUSED`/`UNSERVICED` after fn 47; every wake is `(S3 WAKE) ... KEPT`.
  With the Hyper-V timer enlightenments off, the Windows driver's SR-resume Booter Load fired E15 twice
  (`GSP-RESUME`, `Cold -> Booted -> Running`) and Windows re-programmed the BARs. The GSP falsifier ("GSP REFUSED/UNSERVICED
  on the unload path") did NOT occur; the 60 s desktop falsifier DID, for the reasons below.
- [measured] With the production flags (`hv_time,hv_synic,hv_stimer`) the guest never leaves a timer storm after the wake:
  one-shot synthetic timer re-armed every ~13 us with count = -1404037, 740k HLT exits/s, no BAR restored, no device touched.
  Without them: (a) resume ok then a self-reset 62 s later (no dump), (b) bugcheck 0x5C HAL_INITIALIZATION_FAILED (0x201)
  5 s after the wake, (c) three spins on PM1_STS (0x8101) + HPET/PM-timer reads with the screen black (also with `+invtsc`,
  and with `hpet=off`). The old prime suspect, the `0xefbe0000` carve-out REFUSED leaf, is falsified: it occurs in every
  normal boot.
- [inferred] The common factor is the wake's reset of every CPU/timer/ACPI device (`pc_machine_wakeup`) under i440FX + OVMF
  + Windows 11, not kf3. Not tested: q35, a kernel stack of the wedged guest, a bare-QEMU control (impossible: without a
  GPU driver Windows disables S3, "Graphics").
- Hyper-V enlightenments vs S3: dropping `hv_time/synic/stimer` is necessary to get past the storm but not sufficient and
  costs the TSC page (PM timer/HPET reads exit: ~16k-120k pio/mmio exits/s while waiting). The launcher keeps the
  production flags until a flag set passes 3/3.

## 7. Not done / not verified

- **Windows 11.** Not run (owner). The host-side signature before the fix was identical (README of the
  failure trace). What should now hold: the wake reset is `(S3 WAKE) … KEPT` if the Windows driver's fn 47
  says `bInPMTransition=true`, and its SR_RESUME Booter Load fires E15. Untested Windows-specific parts:
  its FBSR path (WDDM FBSR types) and whether it sends controls this branch does not serve.
  **Retest:** boot run 2990's VM on this branch's binary, `powercfg /hibernate off`, sleep from the Start
  menu (or `rundll32 powrprof.dll,SetSuspendState 0,1,0`), wait for QMP `info status` = suspended,
  `system_wakeup`, then check kf3's log for `(S3 WAKE)`, `GSP-RESUME`, any `GSP REFUSED`/`UNSERVICED`
  after the fn 47, and the desktop/`nvidia-smi` in the guest.
- FSP families (Hopper/Blackwell): the FSM keeps their PM life (E13 + E14 are unit-tested), but their
  `FspBoot` sequence emits no `FirmwareResume`; their SR_RESUME (`kgspBootstrap_GH100`) is unbuilt.
- `FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING` is answered without a host act (`pmsr.rs` says why);
  `[inferred]` that nothing runs between its two calls.
- The control ids in `pmsr.rs` and `chanlink::DISABLE_USERMODE_CHANNELS` are cited from the 580.159.04
  header, not yet in the driver matrix (`tools/drivermatrix/consumed.txt`, needs a regen).
- A cold `system_reset` of a VM while it is S3-suspended ends the kept life (unit-tested in
  `reset.rs`), not run on hardware.
