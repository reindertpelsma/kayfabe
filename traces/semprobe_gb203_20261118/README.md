# kf-semmarker-probe on GB203, 2026-11-18

STATUS: LIVE, 2026-11-24. Evidence for `docs/design/V3_BLACKWELL_USER_RING_ROUTE.md` section 2.7 (is `SET_REFERENCE` the
`Xid 32` METHOD on GB20x, and does the stage-1 marker cell run clean). Labels: MEASURED = a line in a file in this
directory, named with its file; INFERRED = reasoning.

## Run

| | |
|---|---|
| date | 2026-11-18, 05:30:50Z to 05:32:17Z (the run log and the dmesg files) |
| GPU / driver | NVIDIA GeForce RTX 5070 Ti (GB203), 595.91.07, `window.log` stage 0 and `SEMPROBE_SETUP` (`arch=0x1b0 impl=0x3`) |
| channel | USER class `0xca6f`, ring GPU read-only (`SEMPROBE_SETUP ... ring_gpu_read_only=true`) |
| probe | commit `d360cd651` (the first commit of PR #506), built on the box. The verdict function of that commit expected A's witness to be MISSING; see below |
| probe binary | sha256 `f0bdeb65...` as recorded in design section 2.7; the binary is not in the tree and the full hash was not kept |

## What the files show

| variant | attempts | host Xid | word | files |
|---|---|---|---|---|
| A: `SET_REFERENCE 0x4B460001`, then a witness release | 1 (run stopped after it) | one `Xid 32 ... channel 0x00000002 intr 00200000` | witness `0x57170041` landed | `semprobe_gb203.txt`, `dmesg_window.txt` |
| B: the stage-1 semaphore marker cell | 5 (2 canary children in `window.log`, 3 formal children in `children_BC.txt`) | none in `dmesg_children.txt` | `0x4b460001` every time | `window.log`, `children_BC.txt` |
| C: `NON_STALL_INTERRUPT 0`, then a witness release | 3 children | none | witness `0x57170043` every time | `children_BC.txt` |

- **A was measured once (2026-11-18).** The design asked for N=3 per variant, and the plan was one run of three attempts. The
  probe stopped at A's first attempt (fail closed), so A has ONE attempt, on one die, on one driver. B and C were then
  run as separate child processes, outside the parent's fail-closed loop and its dmesg-delta check; their Xid
  evidence is the box dmesg window in `dmesg_children.txt` (the lines after the A attempt).
- **The verdict was revised after this run.** `SEMPROBE_RESULT ... verdict=Unexpected` in `semprobe_gb203.txt` is the
  `d360cd651` verdict, which required the witness to be missing after A's Xid. The witness landed, so the run
  stopped. The verdict in the tree now accepts exactly one Xid 32 with `intr 00200000` whatever the witness. No run
  has used the revised verdict: the tree's verdict function is checked only by unit tests, including a test that
  replays this attempt's Xid line.
- **The `Xid { code: 20, ... }` in `SEMPROBE_RESULT` is hexadecimal** (`{:x?}` formatting): 0x20 is Xid 32, and
  `intr: Some(200000)` is `0x00200000`.
- `dmesg_children.txt` carries the Xid line with the box's wall-clock stamp (`05:30:57`), `window.log` and
  `semprobe_gb203.txt` with the kernel-uptime stamp (`18942.34`).
- The `nvidia-smi -q` before and after the A attempt differ in three lines only (`smi_diff.txt`: Tx/Rx throughput and
  one temperature step). The two full outputs are not kept here.
- The PCI address in the Xid line is `0000:00:07`. INFERRED: that address is typical of a GPU passed through into a
  VM, so the "bare metal" of the design text means "no kf3 guest was running", not necessarily "the box is not itself
  a VM". The files do not say; nothing here depends on it, but a run on another box should record it.

## Provenance of these files

The original output directory (`/workspace/semprobe_out` on the box, cited by the design) is not on the machine that
holds this repository. The files here were recovered on 2026-11-24 from
`/workspace/bench/blackwell_smoke_20261118/semprobe_out` on the coordinator machine, which holds the same run's
text. They are copied unmodified, except for these two changes:
- `window.log` omits the compiler warnings of its build stage (marked in the file);
- `smi_before.txt`, `smi_after.txt` and the rest of that directory are not copied (no information beyond
  `smi_diff.txt`).

No GPU UUID or serial number is in these files (the box's own `nvidia-smi` output had both scrubbed before they were
written).
