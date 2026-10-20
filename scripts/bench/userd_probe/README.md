# userd_probe: read-only GP_PUT / GP_GET view of the Windows USER-ring channels

**STATUS: LIVE (diagnostic tool), 2026-10-20.** Standard-library Python 3, run as root on the VM host.

```
python3 userd_probe.py                       # one snapshot (rows that are behind or flagged)
python3 userd_probe.py --all                 # every live channel
python3 userd_probe.py --watch --tsv probe.tsv        # refresh each second, forever, log to TSV
python3 userd_probe.py --watch 600 --name boundary-kayfabe-3093
options: --pid, --log, --lowmem, --interval, --stuck-after S (2), --lazy-bound N (32)
```

## What it does

* Finds the QEMU process (`-name`), its run directory (from `-qmp unix:<dir>/qmp.sock`) and its guest-RAM
  `memory-backend-memfd` mapping in `/proc/<pid>/maps`.
* Tails `<run dir>/qemu.log` (read only) for `CLASSIFY .. -> Passthrough`, `CHAN-FACTS` (ring entries, guest
  ProcessID, `userd=Some(Sysmem{base,size})`), `Windows USER ring chan C:H BORN .. token T host H`,
  `act free: C:H: passthrough`, and `act disable channels` (per-twin `bDisable` true/false).
* Every second reads the 0x200-byte USERD slot of each live channel from guest RAM with `pread` on a
  **read-only** `/proc/<pid>/mem` descriptor: `GP_GET` at 0x88, `GP_PUT` at 0x8c (`kf_abi::submit::USERD_GP_GET/PUT`,
  `dev_ram.h` dwords 34/35).
* GPA to memfd offset: below `lowmem` it is the identity, from 4 GiB up it is `gpa - 4 GiB + lowmem`. `lowmem`
  is auto-detected by checking which split makes every logged USERD look like a USERD page (small `GP_PUT/GP_GET`
  below the ring size, almost all other words zero). Measured on this host for a 12288 MiB guest: 3 GiB
  (56/56 slots plausible; 2 GiB 35/56, 2.75 GiB 5/56). The first line printed shows the scores.
* Columns: channel, guest pid (from CHAN-FACTS), host channel id, token, USERD GPA, PUT, GET, delta, `behind`
  (seconds PUT != GET continuously), `static` (seconds without any change while behind), `act` (last
  disable/enable act for that host channel: `DIS(age)` = unmatched disable, `EN(age)`, `EN(..)<30s`), flag.
* TSV: `S` rows (behind channels each tick, all channels every 10 s) and `E` rows (births, frees, disable and
  enable acts seen live).

## What it cannot tell you (read this before trusting a row)

1. **The slot it reads is the guest's slot, not the GPU's.** For a Windows USER ring the GPU uses a *private,
   kayfabe-owned, vidmem* USERD and a private GPFIFO (`V3_USERD_RELAY.md`, `V3_WINDOWS_USER_RING_RELAY.md`;
   the birth line says `private-read-only-ring`). The guest writes `GP_PUT` in its slot; kayfabe copies entries
   into the private ring and writes the private `GP_GET` back into the guest slot **lazily** (the lazy-observe
   rule, `kf_chan::user_lazy`, about every 16 turns, and an idle relay may go quiet with a stale `GET`).
   So `PUT != GET` with a small, frozen gap (up to about 16-19 entries here) is the normal resting state of a
   quiet relay (flag `frozen-lazy?`); it does **not** show that the GPU did not fetch. Only a gap larger than
   `--lazy-bound`, frozen, is flagged `STUCK`, and even that is a candidate, not a proof.
2. The decisive pair (the GPU's private `GP_GET` vs kayfabe's private `GP_PUT`) is in vidmem, in a 1 MiB
   storage object per channel (`kf_chan::user_storage`: `USERD_OFFSET` 0xf0000, GET +0x88, PUT +0x8c, ring of
   512 private entries). Each storage is a 1 MiB `rw-s /dev/nvidia0` mapping of the QEMU process and IS readable
   with `pread` on `/proc/<pid>/mem` (measured). v2 reads all of them every 20 ms and prints a mapping-free
   `REAL-BEHIND private storage <addr>: GPU GET=.. PUT=.. frozen Ns` line when GET != PUT for >= `--stuck-after`
   seconds. Which storage belongs to which guest channel is NOT logged: the tool guesses by co-change
   (Jaccard of change events of guest PUT/GET and private GET/PUT), column `J`; unmatched or ambiguous
   channels show no real columns. Use `--no-real` to skip these reads.
3. Unreadable: a channel whose USERD is not `Sysmem` (the row is counted `unreadable`).
4. Liveness is reconstructed from log births and `act free: .. passthrough` lines; a channel freed without that
   line (client teardown) stays listed until its address is reused by a later birth. A USERD address is reused
   by successive channels, so a stale row can show a recycled page: the `nz=` flag (more than 8 non-zero words)
   marks the page as probably not a pure USERD slot.
5. Act times: the log has no timestamps. Acts present in the log when the probe starts show `(?)`; acts seen
   while it runs carry the wall time they were first read (within one tick).
6. A USERD that moves (ring growth, `FIFO_UPDATE_CHANNEL_INFO` switch) is not tracked; the log does not print
   the new address.
7. The probe is racy by nature (two 4-byte loads of a live page); a one-sample difference means nothing.
