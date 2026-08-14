# `traces/w318_gate/` — w318, THE DIRTY GATE

Full reading: `docs/design/w318_the_dirty_gate.md`. Source revision **`c6301a57`** (stamp gate
passed on every boot); bench `vh2`, RTX 3060 GA106, driver 580.159.04, 2026-08-14.

## The two timing boots — same binary, ONE variable (two environment strings)

| tag | `KAYFABE_DIRTY_GATE_PUBLISH` | `_WITNESS` | what it is |
|---|---|---|---|
| `w318off` | `off` | `off` | ★ **THE CONTROL.** Behaviourally master; reproduces w315's shares. |
| `w318on` | `on` | `on` | the measurement |

⊘ **The control is a better control than a master boot would be.** A master binary would also
differ in the two `AddressTable` fields and the `SparseFb` counter this branch adds, and
nothing would say which difference moved a number. Here the *only* difference between the two
boots is two strings in the environment.

Both arms ran `KAYFABE_KFTIME=on` at `N=512`, 12 iterations, through **w315's own unmodified
`w315_floor.sh full`** — so the per-segment table is comparable to w315's by construction.

Files per tag: `run_<tag>_qemu.log.zst` (the device log, ~77 MB raw), `_probe.log` (the
guest's own `ITER`/`GUEST_BSUM` lines), `_serial.log`, `_dmesg.log` (guest), `_hostdmesg.log`
(the per-boot **delta** — 0 bytes is the normal green), `_kvmexits.log` (1 Hz KVM sampler),
and `<tag>.log` (the harness's own report).

## Reading them

```
python3 scripts/bench/w315_align.py     run_<tag>_probe.log run_<tag>_qemu.log   # per-launch
python3 scripts/bench/w315_attribute.py run_<tag>_probe.log run_<tag>_qemu.log   # aggregates
```

⚠ `w315_align.py` needs the **uncompressed** qemu log.

## ★★★ The three greps that carry the finding

```
grep -ao 'DIRTY-GATE .*' <qemu.log> | tail -1               # the fire/skip ratio
grep -aoE '→ published=[0-9]+ refused=[0-9]+' <qemu.log> | sort | uniq -c   # §4
grep -aoE 'rounds=[0-9]+ → bound=[0-9]+ [^ ]*'  <qemu.log> | sort | uniq -c   # §4
```

The last two are the correctness evidence: their non-zero rows are **identical** between the
arms, so the gate removed only doorbells that published nothing and bound nothing.

## The correctness boots

`w318corr_<tag>.log` + `run_<tag>cup3_*` / `run_<tag>r33_*`, driven by
`scripts/bench/relaxation_inert_gate.sh run`, which grades **both planes or no verdict**:
`^CUP3_VAL=43` (libcuda + a real GR launch) and `★     R33 arm 1 COPY` (raw CE, no libcuda,
its own VAS).

⚠ **n = 1 is not a grade here.** w314 measured a ~20 % false-negative rate on a single-boot
cup3 grade on these boxes, so the gated arm is run three times and the control once, all from
one binary.
