# GOP boot display + unload fixes, final box runs — B0, B1, B5 (2026-10-03)

Box: vast 54032077, RTX 3060 (GA106, 10de:2504), host driver 580.159.04 (open); guest Ubuntu noble
6.8.0-142 with 580.159.04; OVMF (Ubuntu's `ovmf`). Branch `v3-gop-unload`. Design and findings:
`docs/design/V3_DISPLAY.md` §4.11.13 (and the STATUS at the top of §4.11). The fixes these runs test are
the review of `v3-gop-unload` (9 findings) and the five minor findings of the `v3-gop` re-review.

Text logs and PNG renders of the console screendumps only (converted on the box with PIL; no executable
came back). QEMU and serial logs are gzipped. Each boot's `run_*_hostdmesg.log` was empty (0 bytes: no
host kernel line during the boot) and is not kept; `DISPLAY_HOST_XID=0` in each `gopfinal_b*.log` is
counted from it.

| dir | kf3 binary | what | result |
|---|---|---|---|
| `run_h/` | `06b307c4` (`kf3-bins/06b307c4`, `run_*_rev.txt`) | B0 (`gop=off`), B1 (`gop=on`, timed shots), B5 (`DISPLAY_HOOK=unload_hook`, `KF3_DISPLAY_TRACE=1`) | **all pass**; B5 `DISPLAY_B5_VERDICT PASS arms=14 failed=0`, lane rc 0 |
| `run_g_harness_fail/` | `445367a8` | the same three, first try | B0, B1 pass; B5's verdict FAILed on ONE arm the harness misjudged (below) — the lane exited rc 3 |
| `standin_f1/` | local, `445367a8` + the `gop_standin.sh` change of `a7feb3ab` | the Secure Boot arms with the deny arms' positive control | 4 PASS, 1 OBSERVED, 0 FAIL; the bite check FAILs as it must |

Commands: `gopfinal_runs.sh <rev> <suffix>` (the box's driver script, kept here): build kf3 with
`scripts/bench/build_kf3.sh`, then `scripts/bench/display/lane.sh` with `KF_FIRMWARE=ovmf KF_DEVICE=kf3`
and `DISPLAY_KF3_EXTRA=gop=on` (B1, B5) / without it (B0), `DISPLAY_HOOK=unload_hook` for B5. Its output is
`run_h/gopfinal_h.log` (and `run_h/gopfinal_build_h.log`). Between `06b307c4` and the commit that adds
this directory only docs and this evidence changed.

## B1 — no black frame between the boot layer and the first armed head (`run_h/b1h/`)

kf3's own lines (`DISPLAY_BOOT_HANDOFF`, printed by `lane.sh` from `run_b1h_qemu.log.gz`, graded):

```
DISPLAY_BOOT_HANDOFF black_frames=0 first_window=[+53588 ms the console shows head 3] lines=3 (expected: black_frames=0)
+50 ms the console shows the BOOT layer (store 0x0, 1920x1080) (core channel FREE)
+53587 ms the console shows no new frame: no armed head has scanned a window yet — the boot layer's last frame stays (no black at the handoff)
+53588 ms the console shows head 3 1920x1080: [window 6 iso 0x10088+0x0 1920x1080 pitch 120 fmt 0xe6]
```

Before the fix the line between those two was *"+52936 ms the console shows BLACK"* (b1f at `4a4b95f7`,
`../gop_unload_20261003/b1f/`); `lane.sh`'s check finds that one in b1f's log. Timed shots
(`shots/t*.png`, `shots/contact_sheet.png`, seconds after the monitor socket appeared): QEMU's
placeholder (1–6 s), the boot layer's zeroed framebuffer before OVMF draws (9–12 s, black, as in every
earlier B1), TianoCore (16 s), the EFI stub (20–25 s), kernel and systemd text (30–40 s), the login prompt
(45–65 s, through the handoff; the shots at 48–58 s are 2 s apart), the probe's pattern (75 s), fbcon
(90 s) — no shot from the handoff window is black. Probe pixel-exact at 1920x1080, 120/120 flips at 60.00 Hz, host Xid 0.
**B2's checks, same boot:** *"the guest preserves a firmware console of 0x7f0000 bytes … 3 regions"*;
*"boot display seed [0x0, +0x7f0000) retired at the first change: the console is ONE run, VA 0 -> store 0,
0x7f0000 bytes"*; `cannot preserve` 0 times in the guest dmesg; `boot[frames=775 retired=+53587ms]`;
nvidia-smi's teardown re-seeded BAR1 (seed life 2) and nvidia-drm's load retired it.
At the end of the boot (+93933 ms, the guest's shutdown) the lit head lost its window: *"no new frame: a
lit head has no window (held up to 250 ms)"*, then *"BLACK … a lit head has had no window past the hold"*
250 ms later.

## B0 — `gop=off` (`run_h/b0h/`)

Probe pixel-exact, 120/120 flips at 60.71 Hz, host Xid 0. No boot layer, no seed, no re-seed (`B0_SEED_LINES=0`).
What `gop=off` now does, measured at `06b307c4` (2026-10-03): *"+50 ms the console shows NOTHING yet"* (QEMU's placeholder) until
*"+53292 ms … head 3"*; at the shutdown the lost scanout is black after the hold (+93702 → +93952 ms),
where before this branch the last frame froze. The BAR1-mode write and fn-47 lines print with
`gop=off` too (bounded; two RM teardowns: t=47.889 s, t=97.157 s) and request nothing.

## B5 — the unload arms, with the verdict (`run_h/b5h/`)

`run_b5h_probe.log`, `DISPLAY_B5_JUDGE` / `DISPLAY_B5_VERDICT`:

| arm | expected | observed | shots |
|---|---|---|---|
| B5C nvidia.ko, no RM client | changed=yes | changed=yes | `hook/b5c_before.png`, `hook/b5c_after.png` |
| B5C2 an RM client holding `/dev/nvidia0` | a holder, changed=yes | holder=1785 changed=yes | `hook/b5c2_*.png` |
| B5C3 after the second teardown | changed=yes | changed=yes | `hook/b5c3_*.png` |
| B5A_SESSION Cinnamon Wayland on simpledrm | up, no Xorg, no fresh Xorg.0.log | up=yes, no Xorg, `xorg_log=absent` | `hook/b5a_x.png` |
| B5A_AFTER_SESSION / B5A_CONSOLE | text console back; it keeps updating | yes / changed=yes | `hook/b5a_after.png`, `hook/b5a_tty_*.png` |
| B5A2_SESSION X11 Cinnamon, NVIDIA X driver, `modeset=0` | up on Xorg, NVIDIA driver, fresh log | up=yes, Xorg running, `xorg_log=fresh`, `NVIDIA(0)` | `hook/b5a2_x.png` (Cinnamon's fallback dialog: the known `GF100_DISP_SW` gap), `hook/b5a2_Xorg.0.log` |
| B5A2_AFTER_SESSION / B5A2_CONSOLE | text console back; it keeps updating | yes / changed=yes | `hook/b5a2_after.png`, `hook/b5a2_tty_*.png` |
| B5A2_PRESERVED | *"the PRESERVED scanout"* | `lines=1`: *"+149612 ms … the PRESERVED scanout 1920x1080 [window 6 store 0x0]"* | |
| B5B_FBCON `modeset=1 fbdev=1` | rc=0, `nvidia-drmdrmfb`, text shown | rc=0, nonblack=3/1000 | `hook/b5b_fbcon.png` |
| B5B_AFTER_RMMOD fbcon unbound, `rmmod nvidia_drm` | rc=0, black | rc=0, nonblack=0/1000 | `hook/b5b_after.png` |
| B5B_DEVICE | kf3 chose black | `lines=1`: *"+207323 ms … BLACK"*, 250 ms after the window left | |
| B5_TEARDOWNS | each PHYSICAL write → `[0, G)` shown again, none refused | physical_writes=5 restored=5 refused=0 | |

The device's account (`hook/b5_device.log`, now complete — no family ran out of lines, no *"lines logged"*
closing line):
- **The handoff** (X's first modeset, in (a2)): *"+133810 ms … no new frame: no armed head has scanned a
  window yet — the boot layer's last frame stays"* then *"+133810 ms … head 3"* — no black
  (`DISPLAY_BOOT_HANDOFF black_frames=0`).
- **X's modeset**: head 3 lit with no window for 20 ms (+149439 → +149459 ms): *"no new frame … held up to
  250 ms"*, then head 3 again — no black; before the fix the same 22 ms was a BLACK frame (b5f).
- **PRESERVE_HW per display object**: (a2)'s frees carry `preserve: true` (window 6, core); (b)'s and the
  shutdown's `preserve: false`.
- **Every teardown** (five RM lives: nvidia-smi, the (c2) holder, the (a) session, X in (a2), nvidia-drm in
  (b)): the console's unmap (*"guest views []; 0x7f0000 bytes show SCRATCH"*), the
  `NV_PBUS_BAR1_BLOCK = 0x0 (MODE PHYSICAL)` write, *"BAR1 back to its physical view … (seed life N)"*,
  fn 47 (*"already shows its physical view"*). For the first four the unmap and the write are in the same
  ms and the re-seed ≤ 1 ms after (t = 48.551/48.552, 76.086/76.087, 115.915/115.915, 149.751/149.752 s).
  The fifth is different: in (b) nvidia-drm's fbdev surface had replaced the console at BAR1 VA 0
  (t = 196.501 s unmap, 196.517 s *"guest views [(0, 7f0000, Some(a00000))]"*); `rmmod` unmapped it at
  t = 210.110 s, 406 ms before RM's teardown wrote the register (210.516 s, re-seed 210.517 s) — with
  fbcon unbound, nothing drew into BAR1 in that window.
- Still seen: one *"Flip event timeout on head 0"* at `rmmod nvidia_drm` (guest 191.0 s,
  `run_b5h_dmesg_after.log`; `DISPLAY_FLIP_EVENT_TIMEOUTS_AFTER=1`), as in every earlier B5 run — open,
  not investigated (§4.11.13).

## `run_g_harness_fail/` — the verdict caught the harness (binary `445367a8`)

The first run of the new hook: 13 of 14 arms PASS and `B5A2_PRESERVED FAIL observed=[absent]`, lane
rc 3. The arm's own `DISPLAY_B5A2_DEVICE console_shows=[…]` line, printed from the same `qsince` output
moments before, lists *"+149539 ms the console shows the PRESERVED scanout"*. The check was `qsince |
grep -aq` under `pipefail`: grep exits at the first match, `tail` takes SIGPIPE on the live log, and the
pipeline fails on a match. `06b307c4` counts matches instead (`qcount`). kf3 was right; the harness was
not, and the new verdict made that visible instead of printing *"(expected: …)"* beside a green rc.
B0 and B1 at `445367a8` passed the same way as in `run_h` (`gopfinal_g.log`).

## `standin_f1/` — Secure Boot, the deny arms' positive control (local, KVM, no GPU)

`scripts/display/gop_standin.sh sb_snakeoil_signed sb_snakeoil_tailpad_observe sb_deny_signed
sb_deny_unsigned sb_deny_tailpad` with `OVMF_DENY_CODE` built by `scripts/display/build_ovmf_deny.sh` from
QEMU 10.2.4's `roms/edk2` (2 min) and Ubuntu's `OVMF_VARS_4M.snakeoil.fd`: the signed end-aligned ROM
verifies and runs (21/21 checks); the unsigned and tail-padded ROMs do not run under the deny policy AND
did run on the stock Secure Boot OVMF with the same keys (`control_on_stock_ovmf=[… rom_started=1 …]`) —
so their not running is the denial. `bite_broken_rom.log`: a ROM with its PE signature destroyed starts
on neither firmware and the deny arm FAILs it by name; the rule before (*"PASS when it did not start"*)
passed it. The log reads `rev=445367a8-dirty`: the script under test was `a7feb3ab`'s change, not yet
committed when it ran.
