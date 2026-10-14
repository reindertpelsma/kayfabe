# Vsync oracle on a real Windows rental (2026-10-14) — requested add-on, mostly NOT done

**STATUS: ANSWERED (partial), 2026-10-14.** Labels: **[M]** measured here, **[U]** not measured.

Request (coordinator): run the two small GL apps (C GL swap interval 1; LWJGL3 Java swapInterval 0/1) from branch
`claude/win11-opengl-20261014` (`scripts/bench/gl/`, host worktree `/data/claude-oglw/wt`) for 90 s each on a real Windows rental and bring back
frame times, driver errors, and the `NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE` (0x0073010c) / `NV5070_CTRL_CMD_GET_RG_SCAN_LINE` (0x5070020c) pairs.

* **The apps did not exist** when the Windows box was up (`/data/claude-oglw/wt` absent; `git log origin/claude/win11-opengl-20261014` has only evidence
  commits, no `scripts/bench/gl/`). Skipped as instructed; no frame times. [U]
* **The rental has no display** (vast KVM guest: only `viogpudo`, no monitor attached to the passed-through GPU) — a windowed GL app on it would run on the
  virtual adapter or fail; not tried. [U]
* What the kfwc captures of the boot (no app, no monitor) do hold [M], same on every driver 572.83 .. 610.88 (GSP firmware 570.133.07 .. 610.57.01):
  * `0x0073010c` (16 B request / 16 B reply, status OK): **28 calls (36 on 590/595)**, request `{subDevice 0, head 0..3, flags 0|1, 0}`, **reply = the request
    echoed unchanged** (no active display on any head): e.g. `{0,0,0,0}` -> `{0,0,0,0}`, `{0,0,1,0}` -> `{0,0,1,0}`. Bodies: `ga106_*/C/*.bodies.tsv`.
  * `0x5070020c` (`GET_RG_SCAN_LINE`): **never issued** (it needs a lit head), so the real-hardware scan-line reply is not obtained. [U]
* What would be needed: a rental with a monitor/EDID dummy plug and a logged-in desktop session, the apps, and the kfwc allow-list already extended for
  both ids (`scripts/bench/constants/process_capture.sh` allows `0x0073010c`, `0x5070020c`, `0x50700117`, `0x5070020d`).
