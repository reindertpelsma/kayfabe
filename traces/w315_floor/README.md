# `traces/w315_floor/` — five guest boots, one variable each

Reading: `docs/design/w315_the_launch_floor_is_our_own_doorbell_handler.md`.
Bench `vh2`, RTX 3060 **GA106**, driver 580.159.04, 2026-08-14.
⊘ **`vh2` is itself a KVM guest** — our guest runs at **L2** and every MMIO access is a
*nested* vmexit. What that costs is measured (§4 of the doc) and it is **not** the answer.

| tag | `KAYFABE_KFTIME` | rev | guest `med_ms` | `submit_med` | `sync_med` | what it is for |
|---|---|---|---|---|---|---|
| `w315base` | *unset* | `72b6f66f` | 111.985 | 88.408 | 23.301 | **THE BASELINE.** Asserted: **0** `KFTIME` lines in the device log |
| `w315census` | `census` | `72b6f66f` | 108.931 | 85.504 | 23.248 | the aggregate breakdown, no per-event lines |
| `w315full` | `on` | `72b6f66f` | 113.562 | 90.865 | 23.391 | **per-event lines — the alignment runs on this** |
| `w315inject` | `census` + 30 ms → `vas_publish` | `72b6f66f` | 149.628 | **126.247** | 23.293 | ★ **THE KNOWN-POSITIVE** |
| `w315full2` | `on` | `9196b8fa` | 116.909 | 93.977 | 22.967 | ★ **the confirmation boot — n=2 on the headline** |

All five: `N=512`, `iters=12`, `batch=10`, `verify` every iteration, `bad=0 maxerr=0`,
(E) VERDICT = 0, **zero Xid**.
⚠ **`bad=0` here is UNGUARDED** — every arm ran `KAYFABE_BENCH_ONLY=measure`, so the
`BENCH_NOLAUNCH` negative control was **not** run in this rung. Correctness is inherited from
w311 (guest `262144`), at a different revision.

## The headline these files carry

```
guest launch_ms                        113.6 ms  100%
  cuLaunchKernel (SUBMIT)               90.9 ms   80%
    inside ONE doorbell MMIO trap       86.7 ms   76%   ← 97.5–98.9% of submit, both boots
    vmexit + guest driver (outside us)  1–2  ms    2%
  cuCtxSynchronize (COMPLETION)         23.3 ms   20%

inside that trap:  vas_publish 55.7% · pt_decode 25.7% · pt_sweep 7.6% · pt_vascensus 2.4%
                   ⇒ page-table + publication  91.5%      (shape=work — bare metal keeps it)
                   core, THE REAL HOST FORWARD  4.1%
                   our own logging              0.2%
                   UNMARKED inside the bracket  0.01%     ⇒ the breakdown CLOSES
```

## Files

- `run_<tag>_qemu.log.zst` — the device's own emissions. **The `full`/`full2` ones are the
  evidence**: they carry one `KFTIME` line per MMIO event with its segment map.
  ⚠ 416 969 per-event lines in `full` ⇒ ~1.4 MB compressed from a ~69 MB log.
- `run_<tag>_probe.log` — the guest half. `ITER … submit_ms= sync_ms= t0_mono_ms=` is what the
  alignment consumes; `GUEST_BSUM` is the summary.
- `run_<tag>_kvmexits.log` — the **1 Hz KVM exit sampler**, host-wide.
  ⊘⊘ **`exits` is per-LIVE-VM and resets to 0 when the VM dies**, so first-minus-last is `0`
  on any completed boot — a number that looks like a measurement. Read the *positive per-row
  deltas*: `w315base` carries **1 681 577 exits / 277 894 mmio_exits, peak 27 434 mmio/s**,
  with `exits=0` at both ends of the same file.
- `run_<tag>_hostdmesg.log` — the per-boot host dmesg delta. **0 bytes on four of five**;
  `w315full` holds one line, `hrtimer: interrupt took 5601309 ns` — the eprintln flood, not an
  Xid. ⊘ A 0-byte delta passes (E1) by having nothing in it; that is a statement about the
  file, and it is why the content is quoted here.
- `run_<tag>_serial.log`, `w315<tag>.log` — the boot console and the rung's own grading block.
- `w315_align_full.txt` / `w315_align_full2.txt` — **the deliverable**, regenerated from the
  two logs by `scripts/bench/w315_align.py`.
- `w315_attribute_census.txt` — the aggregate view plus the vmexit bound.

## ⊘ What is NOT here, and would be needed next

- **A bare-metal control.** 95.6 % of the trap is `shape=work`, so the prediction is that a
  non-nested host changes the answer by ~2 %. Nothing here tests that prediction.
- **A boot with `vas_publish` gated.** This rung deliberately fixed nothing. The 48 ms is
  located, not removed, and no measurement here says what removing it costs or buys.
- **Sizes other than 512, and any workload but this matmul.** The floor is fixed per launch,
  so N=512 is where it is the largest *fraction* — that is why it was chosen, and it is also
  why these absolute numbers must not be quoted for a larger kernel.
