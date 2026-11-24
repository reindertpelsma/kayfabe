# Fill probe: hypotheses and falsifiers for findings F1 and F2, written before the run (2026-11-24)

**STATUS: RESEARCH, 2026-11-24. Written and committed BEFORE any hardware run of this directory.** Results go in
`README.md` beside it; this file is not edited after the first run except by a dated addendum at the end.

Source of the findings: the Phase 1 read of PR #595 (`traces/pc_hwgates_20261124/` on branch `claude/pc-hwgates-20261124`:
pc, RTX 4070, host driver 595.91.07, kf3 `63e46069`). Revision under test here: `origin/next` `4a6a2108e`; this branch adds
only the probe binary `crates/kf-harness/src/bin/kf-fill-probe.rs` and this directory (no product code).

Box: ONE Vast KVM box (`vms_enabled=true`, the KVM image), a real 24 GB RTX 4090-class Ada die (`gpu_ram` about 24564,
not 49140), host driver 595.91.07 (the #595 driver; `windows_lane.sh` default). Caps for both experiments together:
3 USD and 3 box-hours; `vast_guard.py --enforce` before and after; teardown by the box's own id, absence verified.

Labels in the results: MEASURED (a file in this directory) and INFERRED (reasoning, said so).

## Experiment 1: F1, host-only raw-client probe (no VM)

`kf-fill-probe <zero|fault> <floor|default> <alias_len_hex> <churn_regions> <rounds> [extra_spaces]`: an unconfigured
`HostRm` with `set_guest_va_fill` set as named, the 4 GiB store (`reserve_gpga`, as fb-mb 4096), a mirror space started as
named (`floor` = `MirrorVaStart::GuestFloor`, the Windows shape with the third SPARSE range `[64 KiB, 1 MiB)`), optional
churn (`churn_regions` 4 KiB `SharedSlice` maps per round, each mapped and unmapped at once in its own 2 MiB region:
256 per round in `[0x120000000, +4 GiB)`, the rest above 1 TiB), then ONE FIXED 4 KiB-pinned map of the store's first
`alias_len` bytes at `0x120000000`. Host FB used (`nvidia-smi memory.used`) is read at every step; host `dmesg` is
read after each arm (`mmuWalkMap`, `NV_ERR_NO_MEMORY`, `Xid`). The alias length is `0xefa00000` (3.74 GiB, the #595 alias).

Arms, each a separate process, in this order (stop at the first Xid; churn bounded so the PTs it can keep stay under
about 2 GiB):

| arm | fill | start | churn | question |
|---|---|---|---|---|
| A1 | zero | floor | none | (a) fresh space, fill ON |
| A2 | fault | floor | none | (b) fresh space, fill OFF |
| A3 | zero | default | none | the Linux shape, fill ON |
| A4 | fault | floor | 65536 x 4 | churn, fill OFF: memory baseline |
| A5 | zero | floor | 65536 x 4 | (c) churn, fill ON |
| A6 | zero | floor | none, 32 extra spaces | many process spaces (Windows) |

If A1 fails, a length sweep on fresh fill-ON floor spaces (`0x4000000`, `0x20000000`, `0x40000000`, `0x80000000`) finds
where it starts.

**H1-churn (the hypothesis as given, from `mmu_walk.c:1482-1492`):** with the fill ON an unmap re-marks the range sparse,
RM never frees a page table that holds sparse entries, so host page-table memory grows with every VA ever mapped, and the
later alias map fails with `0x51`.
- Predictions: A1 and A2 succeed; A5's FB used grows by about 4 KiB per churned region (about 1 GiB for 262144 regions)
  and A4's does not; A5's alias map fails with `0x51` (NVOS46 status; kf-host names it `VA_ALREADY_MAPPED`).
- **Falsifiers:** (i) A1 fails on a fresh space with no churn; (ii) the alias never fails in any arm; (iii) A5's FB used
  does not grow with churn (growth within noise, under 64 MiB, or no larger than A4's).
- The memory-growth part and the failure part are judged separately: the growth can hold while the failure does not.

**H1-fresh (alternative, written down so a fresh failure is not read after the fact):** with the fill ON, the map of a
large 4 KiB-pinned slice inside a SPARSE reservation needs more page-level memory than RM's per-map pool reservation
(`gvaspaceReserveMempool`, sized for the map length at the 4 KiB worst case) provides, because the sparse parent levels
also need sparse-filled big-page tables (INFERRED from the dual-PDE layout; not read to the line).
- Prediction: A1 fails, A2 succeeds, the failure has a length threshold in the sweep.
- **Falsifier:** A1 succeeds.

**H1-floor (alternative):** the third SPARSE range of a floor-started space is the trigger. Prediction: A1 fails and A3
succeeds. **Falsifier:** A1 and A3 answer the same.

**H1-spaces (alternative):** many fill-ON spaces exhaust the pool. Prediction: A6 fails while A1 succeeds.
**Falsifier:** A6 succeeds.

## Experiment 2: F2, the Windows guest powers itself off with `nvlddmkm` 153 (display off)

Guest: Windows 11 LTSC 2024 from `windows_lane.sh` (the non-proprietary from-spec image of the 2026-11-23 Vast S3 runs),
580.88 with GSP forced, prepared by `windows_gate.sh` (display ON, product defaults). Then two boots of child overlays of the
gate's overlay with `win_vm.sh run --once`, arm B (`host,-vmx` + the Hyper-V flags with `hv-crash`: the #595 CPU),
`guest-driver=580.65.06`, `WINVM_KF3_FB_MB=4096` (the #595 fb-mb, so the F1 alias has the #595 length), product defaults
for every kf3 flag (so `unmapped-reads-zero` is ON, the #580 default, as in the failing #595 boots), the same kf3 binary:
- **D-off:** `WINVM_KF3_DISPLAY=off` (no display plane, no GOP; `-vga none`), the #595 shape.
- **D-on:** `WINVM_KF3_DISPLAY=on` (`display=on,gop=on,gop-efi=<signed kf-gop>`).

`win_vm.sh run` passes `-action reboot=shutdown,panic=none`, so a bugcheck leaves the VM paused with a QMP
`GUEST_PANICKED` event (recorded per boot in `logs/<tag>_b1_qmp_events.jsonl`) instead of #595's silent rc-0 power-off.
That is a deliberate deviation from #595: it is what gives the bugcheck code. Each boot is watched for 300 s after ssh
answers: every 30 s QEMU's status, and the guest's System log (nvlddmkm 153/141, Display 4101, Kernel-Power 41,
User32 1074, EventLog 6008, BugCheck 1001). After it: QMP events (`GUEST_PANICKED`, `SHUTDOWN` reasons), host `dmesg`
(`mmuWalkMap`, `NO_MEMORY`, `Xid`) and the kf3 log lines at `0x120000000`.

**H2a: the display plane being off is the cause** of the 153 series and the power-off.
- Prediction: D-off logs 153 (and dies or panics within 300 s); D-on logs no 153 and stays up 300 s.
- **Falsifier:** D-on also logs 153 or dies.
- Not conclusive either way: D-off stays clean too (then the #595 failure does not reproduce on this die/box, and H2a
  is untested here, said so).

**F1 with the display on:** recorded, not hypothesised: whether host `mmuWalkMap ... 0x51` at `0x120000000` appears in
D-on and in D-off.

## Decision rules written now
- A verdict is per hypothesis, from the falsifiers above; a result outside them is reported as such, not fitted.
- If H2a holds: the finding is "Windows needs display=on"; the doc fix text goes in the README (the howto already says so:
  `docs/howto/requirements.md`, `docs/howto/attach-to-existing-vm.md`).
- Product code is not changed by this experiment, unless a fix is tiny and obviously right (then one GPU-free Rust test
  failing before and passing after, and its security-sensitive hunks listed).

## Addendum 2026-11-24, before any run

Added before the first run, after reading `gvaspaceReserveMempool` (`ogkm-580 gpu_vaspace.c:5272-5332`: the pool reservation
is the page-directory plus page-table size of `[0, size-1)` at 4 KiB pages only, computed from VA 0, not from the map's VA):
the probe's store is the alias length rounded up to 1 GiB (at least 4 GiB), and the sweep gains three arms with the Linux
kernel alias length `0x1efc00000` (7.9 GiB, fb-mb 8192): S5 fill ON floor, S6 fill ON default start (the Linux shape), S7
fill OFF floor. H1-fresh predicts that a fill-ON failure depends on the length (not monotonic is allowed: the reservation is
rounded to 2 MiB pool chunks); the INFERRED size of the extra need is one 256-byte big-page table per 2 MiB region
(about 0.47 MiB for the 3.74 GiB alias, against a margin of about 0.5 MiB). Falsifiers unchanged.

## Addendum 2, 2026-11-24, AFTER arms A1-A6 and S1-S7 ran, before A7/A8

Written after the first results (so A7/A8 are a follow-up, not part of the original design): with the fill ON, the static
range `[1 TiB, 128 TiB)` was NOT reserved in any space (`NoMemory`), so the churn above 1 TiB in A4/A5 went through the
space's non-sparse `NV01` range and only 256 regions per round (1024 in all) churned inside a SPARSE reservation. A7 (fill
ON) and A8 (fill OFF) therefore churn 2 x 131072 distinct 2 MiB regions all inside the first range
`[0x120000000, 960 GiB)` (`low` layout), then map the alias.
- H1-churn, memory part, prediction: A7's FB used grows by about 4 KiB per region (about 1 GiB) and stays after the
  unmaps; A8's does not. Falsifier: A7 grows by under 64 MiB, or no more than A8.
- H1-churn, failure part: falsified already if the alias map in A7 succeeds (it is the same test as A5 with more churn).

## Addendum 3, 2026-11-24, AFTER A7/A8, before P1-P3

A7 kept about 1 GiB of host FB after the churn and the alias unmap (A8: none), but the alias map still succeeded on the
24 GB die. P1-P3 rerun A7, A8 and A1 with VRAM fillers that leave about 3 GiB free after the 4 GiB store
(`KF_FILL_PROBE_LEAVE_FREE_MIB=3072`; the pc's RTX 4070 has 12 GB), to test the failure part of H1-churn under the
headroom of a smaller card.
- Prediction (H1-churn, failure part): P1 (fill ON, churn) fails the alias map with `0x51`; P2 (fill OFF, churn) and
  P3 (fill ON, no churn) succeed.
- Falsifiers: P1 succeeds (the failure part is not shown at this headroom), or P3 fails too (then the fill fails the
  alias without churn, under pressure, and churn is not needed).
