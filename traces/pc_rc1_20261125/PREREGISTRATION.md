# Preregistration, written before any run (pc run 2026-11-25, `237349e0`, tag pc_rc1_20261125)

STATUS: LIVE, 2026-11-25. Design intent and falsifiers only; no result here.

Run: kf3 built from the head of branch `claude/rc1-b2-windows-display-20261125`, revision
`237349e04c38d685e04fc8b84e8e39f25c236795` (the RC base plus #604 and the B2 change), on the owner's pc, RTX 4070, host driver
595.91.07, no GPU bind (kf3 reaches the host GPU through unprivileged RM; the desktop stays up). Method, scripts and image are those of
`traces/pc_hwgates_20261124/` and `traces/pc_displayexp_20261124/`.

## Hypotheses and falsifiers
H-build. The C change of #604 (`DEFINE_PROP_ON_OFF_AUTO` in `qemu/hw/misc/kf3/kf3.c`) compiles. Falsifier: any compile or link error
(recorded exactly; the run then stops).
H-gates. `scripts/bench/v3_gates.sh` passes 9/9 at this revision. Falsifier: any gate not PASS, or a refusal line.
H-fast. The fast suite passes 30/30 (`KF_DEVICE=kf3`, 180). Falsifier: any arm not PASS.
H-b. No QEMU log of the 30 arms contains `through our reservation`, `HELD BY HOST`, `HELD-BY-HOST` or `HELD-BY-OURSELVES`, and host dmesg has
no `mmuWalkMap: Failed`; every arm log has `kf3: unmapped-reads-zero=auto` (the two fault arms `=off`). Falsifier: any such line
(the premise of #604 "a passing run has no in-reservation 0x51" is then false: report arm and VA), or a missing/other `unmapped-reads-zero=` line.
H-Wdef (3 boots, `display=on,gop=on,gop-efi`, no `unmapped-reads-zero` argument). Expect log line `kf3: unmapped-reads-zero=auto`, no host
`mmuWalkMap`, no `through our reservation`/`HELD`, guest alive at 300 s, adapter Code 0, `nvidia-smi` names the RTX 4070, no `nvlddmkm` 153.
Falsifier of #604 (a): a 0x51 or refusal line with the default, in any of the 3 boots.
H-Wfill (2 boots, same plus `unmapped-reads-zero=on`). Expect the host `mmuWalkMap ... 0x120000000` failure and the kf3 line
`kf-host: map 0x120000000+... through our reservation ...: REFUSED, not held by the host`. Falsifier of #604 (b): a `HELD BY HOST` line for
that VA. The guest outcome is NOT predicted; it is recorded.
H-Woff (2 boots, `-vga none -display none`, no `display=on`/`gop=on`). Expect the QEMU log line `kf3: WINDOWS GUEST REFUSED: the guest
declared the Windows driver and this device has no display plane`, the NVIDIA adapter not started (problem code recorded). Recorded:
stability for 300 s, power-off, bugcheck (QMP), number of repeats of the refusal line.
Falsifier of "the refusal is better than the old behaviour (bugcheck 0x113 within 50-170 s, 3 of 3 on 2026-11-24 at `63e46069`)": the guest
bugchecks, reboots in a loop, or hangs at boot. Missing refusal line = the B2 change did not fire.
H-Woffsup (1 boot if time allows, as Woff plus `windows-support=off`). No refusal line expected; recorded only.

## Decision rule (what the owner needs)
B2 stays in the RC if, in both W-display-off boots, the refusal line is present and the guest does not bugcheck, reboot-loop or hang
(a guest that is alive at 300 s, or that ends by a clean poweroff, with the adapter in a problem state, counts as better). If either boot
bugchecks, loops or hangs, B2 is reported as not better than before. Each outcome is reported either way; no inference is written as evidence.

## Rules
Every measurement names its run (pc, 2026-11-25), revision `237349e0` and tag. Scratch only under this run's own directories; delete
nothing else; no bind, no reboot. Stop and report on a build error, a gates failure, or a wedged GPU/driver. Hard stop 15:30Z.
