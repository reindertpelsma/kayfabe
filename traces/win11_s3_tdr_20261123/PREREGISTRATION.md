# Win11 S3 wake on kf3: the post-wake TDR, hypotheses and falsifiers written before the run (2026-11-23)

**STATUS: RESEARCH, 2026-11-23. Written and committed BEFORE any hardware run of this directory.** Results go in
`README.md` beside it; this file is not edited after the first run except by a dated addendum at the end.

Baseline (measured 2026-11-23, Vast box 62082978, RTX 4090, kf3 `1052e2f9`, `traces/win11_s3_wake_20261123/README.md`):
34-45 s after every wake (runs R1, R2, R3; arm A and arm B; with and without the F1 QEMU hunk) the Windows driver logs three
`nvlddmkm` event 153 records (Resetting / Reset / Restarting TDR), its RM unloads (`fn 47 ... newLevel=0`) and comes back on
a new GSP life. Desktop and `nvidia-smi` recover.

## Phase 1, GPU-free (read, not executed on hardware)

Read 2026-11-23 from `traces/win11_s3_wake_20261123/r2_armA_stable_tsc/r2_wake_to_unload.txt.gz` (2709 lines, the kf3 log of
R2 from the `(S3 WAKE)` line to the guest RM's unload), the kf3 sources at `origin/next` `90a658c3`, and `third_party/ogkm-580`
(580.159.04).

- [read] "code 76" is the RPC function id `GSP_RM_CONTROL`, not a control. kf3 answers a control no policy serves with
  `NV_ERR_NOT_SUPPORTED` (0x56) and logs `GSP rpc UNSERVICED` (`kf-gsp` `boot.rs`, `answer`).
- [measured R2, read from the log] the 22 UNSERVICED lines right after the wake are seven controls:
  `0x20800a80` INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO, `0x20800a3a` INTERNAL_GR_SET_FECS_TRACE_WR_OFFSET,
  `0x20802a0f` INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE, `0x2080017e`, `0x2080013f` (not in the 580 headers),
  `0x2080205b` PERF_SET_POWERSTATE, `0x2080080b` BIOS_GET_UEFI_SUPPORT. All seven are refused at a NORMAL Windows boot too
  (`traces/die_coverage_20261104/blackwell/gb202/windows_dcgb202r3/gsp_signals.txt`, lines 30-66), where the driver works.
  [inferred] they are the guest RM's state-load sequence, re-run at the resume, and not wake-specific. The later
  UNSERVICED lines are `0x00730280`/`0x007302a5` (display, not in the 580 headers) and four `0x906f0102`
  (`NV906F_CTRL_CMD_RESET_CHANNEL`) on channel `0xc1d00029:0xff04000e`: the RESET_CHANNEL calls are the driver's TDR
  recovery of that channel, i.e. AFTER the hang, not its cause.
- [measured R2, read from the log] the channel the driver resets, `0xc1d00029:0xff04000e` (passthrough, host 0x37, a
  user-ring relay in Scan mode), was re-enabled after the wake (`DISABLE_CHANNELS(bDisable=false)`) and at its first serve
  logged `USER_RING_REFUSED 0xc1d00029:0xff04000e host 0x37: USER_RING_PUT_READ: Topology; no completion/reuse credit
  [#3]`; two more channels born before the wake refused the same way (`#4`, `#5`: hosts 0x3c, 0x2003d) when re-enabled. A
  refused relay is dead for good (`userpump.rs` `user_ring_serve`: `r.dead = Some(why)`): the channel's work never reaches
  the GPU again. The 32 `MISALIGNED_LEAF` walk samples in the same window (space 0x1efa10000) are the per-process sample
  cap's first 32, so that space had no refusal before the wake [inferred from the cap, not separately measured]; noted,
  not ranked.
- [read] `Topology` on the PUT read comes from the sysmem USERD lease (`userdram::Userd`): it captures the `RamMap`
  generation at the channel's birth and refuses every access once it moved. `RamMap::add` and `RamMap::del` moved it on
  EVERY guest-RAM section added or removed by QEMU's memory listener (`kf3_region_add/del`), whatever the section. The
  user-ring pump's source snapshots (`userread`) and MemoryList leases capture the same generation.
- [read, QEMU 10.2.4 tarball verified by `scripts/ci/qemu_source.py`] the wake runs `qemu_devices_reset(RESET_TYPE_WAKEUP)`;
  `mch_reset` (`hw/pci-host/q35.c:539`) puts SMRAM/ESMRAMC back to their defaults and calls `mch_update`, which toggles the
  SMRAM and TSEG windows; the firmware's S3 resume programs them again. `flatview_simplify` (`system/memory.c:337`) merges
  adjacent ranges of the same RAM region, so opening or closing a window re-splits the low-RAM section: QEMU removes it and
  adds a section of another length that maps the SAME file bytes at the SAME addresses. [inferred] every S3 wake therefore
  calls `kf3_ram_del`/`kf3_ram_add`, which moved the generation and revoked every lease born before the wake; the relays
  of the Windows driver's long-lived channels die at their first access after the wake. (The Linux S3 runs of 2026-10-14
  predate the user-ring relay and its generation check.)

## Hypotheses (ranked), each with its falsifier

- **H1 (the RAM-topology revocation kills the relays born before the wake; the TDR is the Windows watchdog on such a
  channel).** Fix candidate **F2** (this branch): `RamMap`'s generation counts changes of the guest-RAM *translation*
  (guest-physical address to backend file offset), not section events: an added fd-backed section that maps a byte or an
  address differently from any translation registered before revokes; a re-split of the same linear mapping, and a removal,
  do not (every access still resolves its span under the read guard, so a removed range is refused there). kf3 logs each
  `RAM add` / `RAM del` with "translation kept" or "CHANGED ... revoked".
  *Predictions with F2:* at each wake kf3 logs at least one `RAM del` and one `RAM add ... translation kept`, no
  `translation CHANGED`; no `USER_RING_REFUSED ... Topology` after any wake; no event 153 and no `fn 47 ... newLevel=0`
  in the cycle window.
  *Falsifiers:* (a) no `RAM add`/`RAM del` line between a `(S3 WAKE)` and the next USER_RING enable (the wake does not touch
  RAM sections: the old refusal had another cause); (b) a `RAM add ... translation CHANGED` at a wake (the wake really
  moves a translation: F2's premise is wrong); (c) event 153 still follows a wake with no `USER_RING_REFUSED ... Topology`
  in that cycle (H1 may be true but is not the whole cause: then H3/H4).
- **H2 (an RPC left UNSERVICED after the wake starves the guest; code 76 = the lead).** Already contradicted GPU-free (the
  same seven controls are refused at every boot). *Falsifier on hardware:* with F2, the same UNSERVICED set follows every
  wake and no event 153 occurs.
- **H3 (other per-channel state lost at the wake: a kept relay's GP_GET/reference words or the host twin's runlist state
  diverge from what the resumed driver expects).** *Prediction if H3 holds:* event 153 with F2 and no `USER_RING_REFUSED`,
  and a stalled relay (USER_RING_LAZY_OBSERVE / doorbell ledger of the reset channel shows no forwarded work after the wake).
- **H4 (display or BAR re-programming race after the wake).** *Prediction if H4 holds:* event 153 with F2 and kf3 display
  STALL / flip timeout lines, or a `0x00730280`/`0x007302a5` refusal immediately before it.
- The 34-45 s delay is not explained by any hypothesis here; [inferred] it is the time until the driver first waits on a
  fence of a dead channel plus the 2 s TDR delay. Not tested.

## Definition of a CLEAN cycle (fixed before the run; the 2026-11-23 definition plus event 153)

QMP `query-status` reaches `suspended` within 120 s of the suspend request (a scheduled task in the signed-in session, as
before); after `system_wakeup` the guest's ssh answers within 150 s with the SAME `LastBootUpTime`; kf3's log gains
`(S3 WAKE) ... KEPT` and at least one `GSP-RESUME`, and no `device reset #N: vCPUs out`; `nvidia-smi` names the GPU; and
after waiting until at least 120 s past the wake, the guest's System log for the window since this cycle's suspend request
holds no event 41, 1001 or 153. The release-gate reading of `windows-sleep-wake` (docs/RELEASE_CLAIMS.md) is N >= 3
consecutive CLEAN cycles in one boot.

## Run plan and stop rule (time-box: 3 box-hours, 5 USD, ONE box; caps `kf-s3wake-` in vast_caps.json)

1. One 24 GB RTX 4090-class box, `vms_enabled=true`, KVM image, global search by total price; `df -h /`; host clocksource
   recorded. If the L1 TSC is unstable, reboot with `tsc=reliable clocksource=tsc` (the storm path needs a valid TSC page;
   with F1 in the product build it should be clean either way).
2. `windows_lane.sh` then `windows_gate.sh` (arm B, `KF_WINGATE_IDLE_S=120`) at this branch's revision (F2 + the F1 hunk
   from #572). No S3 claim on a red gate.
3. **T1:** arm B, q35, S3 enabled, 3 cycles in one boot, F2 build (`traces/win11_s3_tdr_20261123/s3tdr.py`). Judges H1-H4.
4. **T0 (control, only if box time and money remain after T1):** the same box and overlay, 1 cycle with the
   `origin/next` `90a658c3` build (no F2). *Prediction:* event 153 and `USER_RING_REFUSED ... Topology` after the wake,
   as in R1-R3. *Falsifier of "F2 is what made T1 clean":* T0 is CLEAN too.
5. Stop, compare, write the README. Destroy the box by its own id, verify absence.

## Addendum 2026-11-23 (after the runs of this directory)

The open second cause (display window channel 1 stopped after the wake, H4) continues in
`traces/win11_s3_display_20261123/PREREGISTRATION.md`, written and committed before its runs.
