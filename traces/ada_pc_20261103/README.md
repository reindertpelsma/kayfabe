# Ada regression of `next` 70e3fabd8 (after the Blackwell merge #239) on the owner's PC

STATUS: RESEARCH, 2026-11-03. Evidence for one revision on one die. The hypotheses and falsifiers were committed
before the first run (`PREREG.md`, commit 5aa5006ec). Every number below is MEASURED unless marked INFERRED.

**Host:** the owner's trusted PC: RTX 4070 (AD104, `10de:2786`), 12 GiB, 29 GiB RAM. **Host driver 595.91.07**, not
580.159.04. Kernel 7.0.0-34, IOMMU group `DMA-FQ`. Before the run no QEMU was running and the owner's VM was off.
**Under test:** `70e3fabd8`, which is `next` after #239. **A/B control:** `9297ffdd9`, the first parent of that
merge (`next` before #239), on the same host, with the same harness, fast guest, Windows image and procedure.
Only the QEMU binary was swapped.

## Verdicts

| question | result |
|---|---|
| GPU-free (`cargo test --no-fail-fast` over every `kf-*` crate) | **PASS**: `UNIT rc=0 passed=4375 failed=0 ignored=22` |
| `v3_gates.sh` bare metal | **PASS**: `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, `V3_GATES_BIRTHS births=12 user=12 refused=0 ok=1` |
| `build_kf3.sh` | `KF3_BUILT .../kf3-bins/70e3fabd/qemu-system-x86_64 rev=70e3fabd` |
| fast suite, 30 arms, 180 s budget (host-mode fast guest, guest driver 595.91.07) | **PASS**: `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30 FAST_SUITE_RC=0` |
| host Xid during the Linux runs | 5 per suite: exactly the predeclared, admitted shapes. `defer-liveness`: 4x Xid 31 CE `FAULT_PTE VIRT_READ` @ `0x60_/0x62_/0x62_/0x64_00010000`. `missing-page-fault`: 1x `FAULT_PDE VIRT_WRITE` @ `0xa_11000000`. The pre-merge control produces the identical 5, and so did the 2026-10-25 hc1 run (`host_xid_lines.txt`) |
| Windows 11 guest, boot `w1` on `70e3fabd` | **guest passes** every criterion in `PREREG.md` H3 (table below). 0 resets in this boot |
| flip stall | **not seen.** DWM flipped at `kms=59.9`/s for about 90 s before and about 90 s after a 600 s idle (~5,400 completed flips each time; Turing stalled at 30-40). 0 `STOPPED`, 0 `STALL`, 0 flip-timeout lines |
| walk completion change (H4) | **latency regression, MEASURED. CPU flat.** Details are under *Walk completion* |

The repo's `windows_gate.sh` (PR #238) was **not** used. It is written for a rented box: it installs Windows 11 LTSC
fresh from pinned media in every run. Instead, this run used the PC's recorded Windows procedure, the hp1/hc
recipe of 2026-10-25:
- a fresh CoW overlay on the read-only `/data/hp1-20261025/baseline3/windows.qcow2` (the owner's Windows 11 Pro
  26200 apps image, NVIDIA 580.88, GSP keys set);
- the owner's production argv and flags of run 4501;
- a Wayland broker window, so the display was on;
- a loopback PowerShell agent.

The gate's own `gate_state.ps1` and `gate_run.ps1` (branch `claude/windows-gate-20261101`) ran inside the guest.
The probes were cross-built from the `70e3fabd8` sources (`windows/probes.sha256`).

## Windows boot w1 (next 70e3fabd), `windows/w1/`

Desktop: `GATE_DESKTOP user='kayfabe-windows\kfadmin' dwm=1 explorer=1`. Adapter: `GATE_DISPLAY name='NVIDIA GeForce RTX 4070' driver=32.0.15.8088 code=0 status=OK pnp=OK`.
The adapter was still at Code 0 at every one of the ten idle polls and after the run.
`nvidia-smi -L`: `GPU 0: NVIDIA GeForce RTX 4070 (UUID: GPU-<scrubbed>)`. `-q` worked before and after.

| probe | before idle | after 600 s idle |
|---|---|---|
| `cup8` | `CUP8 VERDICT: PASS`, `bad=0 maxerr=0` | `CUP8 VERDICT: PASS`, `bad=0 maxerr=0` |
| `kf_dxprobe d3d11` | `KFDX RESULT OK`, compute `ms=3078` | `KFDX RESULT OK`, compute `ms=16` |
| `kf_dxprobe d3d12` (reported) | `KFDX RESULT OK`, `ms=62` | `KFDX RESULT OK`, `ms=156` |
| `cup3` (reported) | rc=0 | rc=0 |
| `kf_glgears`, 90 s, windowed in kfadmin's session | `frames=388746 fps=4319.4 KFGL RESULT OK` | `frames=356787 fps=3964.3 KFGL RESULT OK` |

**Reset counters** since the guest's boot (`steps/27_tdr.txt`):

```
display4101=0 nvlddmkm153=0 kernelpower41=0 eventlog6008=0 bugcheck1001=0 livekernelevent_other=78 livekernelevent193=0
```

The count of 78 is not from this boot (MEASURED, `steps/28_wer.txt` and `steps/31_lke_events.txt`):
- All 78 Application 1001 events are `P1=141, Report Status 2049`.
- They name only 26 distinct report ids.
- All 26 are reports already queued in the image's `ReportQueue`, with event times from 2026-10-20 to 2026-10-22
  (earlier sessions on this image chain). WER resubmits them at each boot.
- No live dump was written after boot. The newest dump is from 2026-10-21.

The pre-merge control boot shows the same `livekernelevent_other=78`. On this inherited image the gate's raw
LiveKernelEvent counter cannot separate stale queued reports from new ones. It needs a by-report-time filter
before it is used on any non-fresh image (harness lead, not fixed here).

Stop: guest shutdown through the agent (`powerdown.json` `how=agent-shutdown rc=0`). The backing image was
unchanged (`backing-after.json`), and there was no new host Xid (`xid-after.json` 10 -> 10).

## Control boot w2pre (pre-merge 9297ffdd), `windows/w2pre/`

Same result:
- Code 0; `cup8` PASS before and after the idle.
- d3d11 OK: `ms=188` before, `ms=16` after. d3d12 OK.
- gears `fps=3328.4` before, `3105.6` after.
- `kms=59.9` while gears ran; 0 STOPPED or STALL lines.
- The same stale `livekernelevent_other=78`.

The d3d11 figure of 3078 ms before the idle in w1 is n=1 and early in boot: the agent answered 9 s after QEMU
start in w1 and 30 s after in w2pre. After the idle both boots give 16 ms. INFERRED: boot-time contention. It is
not attributed to the merge.

## Walk completion: host callback replaced by `cuEventQuery` polled at 1 ms (`kf-cuda` `completion.rs`)

**MEASURED, same host, same harness, binary-only A/B** (`linux/ab_walk_compare.txt`, `windows/*/host_summary.txt`):

| | next 70e3fabd | pre 9297ffdd |
|---|---|---|
| Linux: per-walk latency `vat[walk_avg_us]`, every arm with walks | **1008-1011 us** | 106-226 us |
| Linux: GPU time `gpu_avg_us` | 81-199 us | 82-199 us (unchanged) |
| Linux: MMU-invalidate arrival-to-clear (`concurrency` arm) | 1029 us | 152 us |
| Linux: suite client time, sum of 30 arms | **128,352 ms** | 67,176 ms (**1.91x**) |
| Linux: `concurrency` / `ce-client-guest-ram` / `concurrent-fuzz` / `uvm-mean` | 14,560 / 62,890 / 3,350 / 1,270 ms | 3,370 / 20,940 / 1,290 / 619 ms |
| Windows: walk latency (weighted, 25k / 24k walks) | **885 us** | 169 us |
| Windows: invalidate arrival-to-clear | 1199 us | 390 us |
| Windows: `kf3-vamgr` CPU, idle 594 s | 0.45 % of one core | 0.46 % |
| Windows: `kf3-vamgr` CPU, while gears ran | 1.09 % | 1.46 % |
| Windows: whole QEMU CPU, idle | 153.7 % | 177.1 % |

H4's falsifier (walk latency staying at or below 200 us) is **not met**: the latency rose as predicted.

Root-cause lead (CODE READ plus the numbers above): the VA manager (`kf-qemu/src/device.rs`, the
`poller.wait(PollTimeout::Millis(completion_poll_ms()))` loop) has no wake source left for a finished walk. The
per-walk `cuLaunchHostFunc` eventfd signal is gone, and the nonstall-interrupt wake named in the `walk.rs` comment
evidently does not fire for walks. Each walk is therefore collected only when the 1 ms timeout expires, whatever
its GPU time (~85 us). That puts about +0.9 ms on every walk, and so on every guest MMU invalidate, map and unmap.

The guest's correctness is unaffected: Linux is 30/30 and Windows passes. Map-heavy work is 2-4x slower on Linux,
and the guest's invalidate round trip is about 3x longer on Windows. CPU cost is flat, because idle is still the
50 ms backstop.

Candidate fixes, not tried:
- a sub-millisecond deadline (timerfd or `ppoll` with ns) derived from the measured `gpu_avg_us`;
- a short bounded spin of `cuEventQuery` after submit;
- keeping the eventfd as a wake hint only. The new `ready()` already refuses to treat the hint as proof.

## Six GSS controls with stricter size checks

On host 595.91.07 the guest-visible refusal set changed slightly (diff of the final `gsp_refusals[...]`):
- refused only on next: `fn76` `0x2080852b`, `0x2080852c`, `0x20809064` (1624x);
- refused only before: `0x20808524`, `0x2080852a`, `0x20809004` (1205x), `0x2080a084`, `0x2080a088`, `0x2080a097`, `0x2080a70a`.

The host-side query of `0x2080852a` is now refused: `body is 1716 bytes, its layout is 1784`. The 595 branch
changed that control's size.

No probe or app result changed: the guest passed in both builds. INFERRED: the 595 rows need re-measuring for
these controls on AD104. The host driver here is 595.91.07, not the 580.159.04 named in the task.

## Harness notes (both fixed before any guest result, neither is a negative boot)

1. The first fast-suite attempt was `NOTRUN=30`. My validate script set `CARGO_TARGET_DIR` but did not pass
   `CLIENT=` to `build_fast_guest.sh`, which looks for the raw client under `<repo>/target`. Fixed and re-run;
   both attempts are in `validate_summary.log`.
2. The first Windows launch stopped before QEMU started (`BROKER_EXIT`): the run directory was not
   group-writable for the broker user. Fixed with `chmod 0775`, then relaunched.

Negative Windows boots: 0. Windows boots: 2 (w1 under test, w2pre as control).

## Files

- `linux/next-70e3fabd/`: validate summary, gates log, unit result lines, suite table, per-arm logs (tgz).
- `linux/pre-9297ffdd/`: the control suite.
- `linux/ab_walk_compare.txt`: the per-arm A/B.
- `windows/<run>/`: `guest_summary.log` (one line per step), `steps/` (raw guest output), `host_summary.txt`,
  `qemu.log.gz`, `cpu.jsonl.gz` (per-thread CPU every 10 s), the scrubbed `command.json`, and screenshots.

GPU UUIDs and the VM identity (`-uuid`, SMBIOS serial, MAC) are replaced with `fa4efa4e-...` placeholders.
Scripts used (host-local, not product): `ada_validate.sh`, `ada_pre_ab.sh`, `ada_win.py`, `ada_guest.sh`,
`agent.py`, `gears_it.ps1`, `ada_win_summary.py`, `ab_compare.py`, `cpu_phase.py`, kept in `tools/`.
