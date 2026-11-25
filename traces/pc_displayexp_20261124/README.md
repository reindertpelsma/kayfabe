# pc display-plane experiment (H2a): Windows `win11_base` under kf3 with and without the display plane

STATUS: RESEARCH, 2026-11-24. Run on the owner's pc 19:57Z to 20:34Z, kf3 revision **`63e460694ea1f268ecf66c7dbe584e208a26d7b7`**
(`kf3-bins/63e46069`, the binary of PR #595), host driver 595.91.07, RTX 4070, guest Windows 11 Pro 25H2 with NVIDIA 580.88 and no apps.
Preregistered in `PREREGISTRATION.md` (committed before the first boot). Labels: MEASURED = a file in `runs/` named here;
INFERRED = reasoning, not seen in a log.

## Setup (MEASURED, 2026-11-24, `63e46069`)
Nine boots, strictly serial, no GPU bind (kf3 used the host GPU through unprivileged RM, the desktop stayed up). Each boot is a fresh
throwaway thin child overlay of the #595 `prep.qcow2` (itself a child of the sealed `win11_base`); the backing files were never opened for
write, and the overlays were removed after each boot. The argv is the #595 r8 argv (arm B CPU flags with `hv-crash`, 8 vCPU, 8192 MiB,
`fb-mb=4096`, `guest-driver=580.65.06`, S3 off, network off, no `-action`) plus a second QMP socket read by `qmp_events.py`.
- **A**: no display plane (`-vga none -display none`), as #595.
- **B**: A plus `display=on,gop=on,gop-efi=<signed kf-gop of win11_base>,display-broker=<sock>,display-broker-uid=0`, with the headless
  `nvkvm-display-broker --backend test` that the 2026-11-18 clone boots used (`propimage_check.py`).
- **B-off**: B plus `unmapped-reads-zero=off`.
Order A1 B1 A2 B2 A3 B3 Boff1 Boff2 Boff3. Each boot was polled over ssh every 10 s until 300 s after the QEMU start or the QEMU exit.
The scripts that ran are `displayexp.py`, `qmp_events.py`, `chain.sh`; the table is made by `summarize.py` (`runs/summary.json`).
The `START`/`END` lines this harness wrote to the pc's `PC_ACTIVE.log` carry the tag `vfio-s3` (the tag of the helper it reuses); nothing was bound.

## Results (MEASURED, 2026-11-24, `63e46069`)
`nvlddmkm 153` is the count of System events 153 from the provider `nvlddmkm` at the last poll that answered. Every boot also has one
Kernel-Boot 153 ("Virtualization-based security ... disabled") that is not counted (see Harness notes).

| boot | arm | alive at 300 s | `nvlddmkm` 153 | bugcheck (QMP `GUEST_PANICKED`) | QEMU end | host RM `mmuWalkMap ... 0x51` (F1) |
|---|---|---|---|---|---|---|
| A1 | no display | no | 6 | 0x113, P1 0x2b | guest-panic, poweroff, at 90 s | yes |
| A2 | no display | no | 6 | 0x113, P1 0x2b | guest-panic, poweroff, at 170 s | yes |
| A3 | no display | no | 4 | 0x113, P1 0x2b | guest-panic, poweroff, at 50 s | yes |
| B1 | display on | yes | 0 | none | killed by us at 300 s | yes |
| B2 | display on | yes | 0 | none | killed by us at 300 s | yes |
| B3 | display on | yes | 0 | none | killed by us at 300 s | yes |
| Boff1 | display on, fill off | yes | 0 | none | killed by us at 300 s | no |
| Boff2 | display on, fill off | yes | 0 | none | killed by us at 300 s | no |
| Boff3 | display on, fill off | yes | 0 | none | killed by us at 300 s | no |

- Arm A reproduced the #595 failure 3 of 3: event 153 from `nvlddmkm` from about 12 s on, then a QMP `GUEST_PANICKED` with Hyper-V crash
  arguments (arg1 = 275 = 0x113, which is `VIDEO_DXGKRNL_FATAL_ERROR` as far as I know it; its P1 0x2b was not looked up) followed by `SHUTDOWN`
  reason `guest-panic` (action `poweroff`). So the earlier silent power-off is a guest bugcheck 0x113 and the QEMU default panic action ends the VM.
  No `Kernel-Power 41`, 1074, 6008 or 1001 was counted before the VM ended (a bugcheck ends it before the next poll; 1001 is written at the next boot).
- Arms B and B-off (6 of 6): `nvlddmkm` 153 = 0, 141 = 0, no Kernel-Power 41, 1074, 6008, 1001; adapter `NVIDIA GeForce RTX 4070`
  ConfigManagerErrorCode 0, `Get-PnpDevice` status OK; a second display adapter `Microsoft Basic Display Adapter` with status Unknown
  (`runs/B*/pnp_at_end.txt`); QEMU alive and the guest answering at every poll up to 300 s. The guest was alive only until 300 s: nothing is
  measured beyond that.
- F1 (host RM page-table allocation failure): one `mmuWalkMap: Failed to map VA Range 0x120000000 to 0x20f9fffff. Status = 0x00000051` per boot,
  followed by `NV_ERR_NO_MEMORY` assertions (`runs/*/host_dmesg_window.txt`), in 3 of 3 arm A and **3 of 3 arm B** boots, and in 0 of 3 B-off boots.
  So F1 also happens with the display plane on. A boot with F1 and display on stays alive (B1-B3); the #595 boots r2 and r7 (fill off, no display)
  died with 153, so F1 is neither necessary nor sufficient for the death in these data.

## Verdict on H2a
Preregistered falsifier: an arm B boot also shows `nvlddmkm` 153 or dies. **Not met: 0 of 3 B boots (0 of 6 with B-off) showed it.**
H2a is supported on this pc: with the same binary, die, image and overlay, adding the display plane turned 3 of 3 failures (event-153 loop,
bugcheck 0x113 at 50-170 s) into 0 of 6 failures over 300 s. Limits: INFERRED, not tested here, which part of the display plane matters
(`display=on`, the GOP driver at boot or the connected broker; they were switched on together); one die, one binary, one guest image; 300 s windows;
no A-arm control with the fill off. Not shown: why a display-less kf3 guest fails (bugcheck 0x113 names the display kernel; the mechanism is not read).
Consequence if H2a is the story (INFERRED): a display-off Windows guest on kf3 is not a supported configuration of this image; the Vast Windows gate
that idles 300 s clean runs with the display on (not re-checked here).

## Harness notes
- The first five boots (A1, A2, A3, B1, B2) counted every event 153 and the poll text does not separate providers; the last poll's event list
  (`events_last_successful_poll.txt`) shows the one non-`nvlddmkm` 153 (Kernel-Boot, VBS disabled), so `summarize.py` subtracts 1 for those boots.
  From B3 on the poll counts `nvlddmkm` 153 only (`153=` field, plus `153KernelBoot=`). The first two boots' `dmesg_filtered.txt` and the
  `host_*` fields of `result.json` are wrong (a line offset into a ring buffer): use `host_dmesg_window.txt` and `summary.json`, which cut `dmesg` by its timestamp.
- `qemu_refusal_histogram.txt` replaces the 1.2 MB refusal-line files (normalised lines, top 25).
- The kf3 log has no line that names F1; the host `dmesg` is the F1 evidence.

## Kayfabe findings (not fixed here)
- F1 is not a display-off artefact: it happens in every default-fill Windows boot of this pc, display on or off (`runs/B*/host_dmesg_window.txt`).
  The alias is then booked HELD and reads zero (a CODE reading of the 2026-11-24 findings note, not re-measured here).
- A display-off Windows guest ends in bugcheck 0x113 on this pc (`runs/A*/qmp_events.jsonl`). No kayfabe code was changed.

## Disk and idle
`/workspace` had 29-30 GB free throughout (thin overlays, 10 MB of scratch kept until the end). All scratch (`bench/displayexp_20261124`, the overlays
dir, `/tmp/dx_*`) removed; no qemu, swtpm or broker process left, `pc_gpu.lock` free, host driver 595.91.07 loaded, `gdm` active.
