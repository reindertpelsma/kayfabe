**STATUS: DESIGN-ONLY, 2026-11-12 (the plumbing exists and is GPU-free tested; it has not run anywhere that matters).**
CODE READ and GPU-free tested only (`scripts/ci/test_broker_ci.py`, `scripts/ci/broker_ci_mutants.py`). NOTHING in this document has run on a Vast box, on the PC or in a VM through the
payload. Every number marked UNMEASURED is a guess to be replaced by the first run. Owner rulings of 2026-11-12 are the
source of the scope (broker in the middle and large tiers; a broker failure is a failure).

# The display broker in the CI box run

## Ruling and what it means for grading

1. **The broker is part of the MIDDLE and LARGE tiers; small stays without** (owner, 2026-11-12). The guests the
   existing GPU stages already boot (Linux fat-guest CUDA ladder and app matrix, the Windows gate, the Windows apps, the
   SPEC lane guests) run **with the broker attached**, so every existing GPU test also exercises the broker path. A short
   explicit stage then asserts the broker's behaviour on its own.
2. **If the broker fails, the test FAILS** (owner, 2026-11-12). The `BROKER_*` classes below exist to *name* a failure
   for diagnosis. Each one grades as an ordinary FAIL attributed to kayfabe. **None is ever a RETRY, operator, provider,
   UNATTRIBUTED or NOT_RUN class, none is excused, none is skipped**, and a broker that does not start, crashes,
   attaches late, shows no frame or loses the guest display fails the stage and the tier verdict exactly like a guest
   failure. The one carve-out is the existing one: a `ProviderFault` (a rented box unusable before kayfabe tests; the
   box itself unreachable) keeps its provider classification, because it is verifiably outside kayfabe. A stage-owned
   refusal that happens before any guest could start (`*_retry_media`, `*_retry_time`, `*_retry_no_list`,
   `*_retry_refused`) also keeps its class, but only while the broker was healthy and saw nothing; with a dead,
   erroring or unstarted broker the broker class wins (`vast_payload.broker_class`).
3. A broker failure is **not a provider failure** and never hides as a generic guest failure: it has its own suite row
   name (`broker_*`, below) and wins over a guest error in the stage's verdict row while the guest's own failing row stays
   in the result (`vast_payload.broker_guarded`).

## What changed (code)

| Piece | File | What |
|---|---|---|
| lifecycle, parsing, grading | `scripts/ci/broker_ci.py` | `BrokerSession` (start/stop/FIFO), log parser, `BROKER_SUMMARY` line, `failure_class`, explicit-stage runner |
| explicit stage guest side | `scripts/ci/broker_explicit_guest.py` | boots a Windows guest through `win_vm.sh`, screendump stats, resize, in-guest logger |
| in-guest logger and resolution tool | `scripts/bench/display/win/{kf_inlog.c,res.ps1,runi.ps1}` | copied from the PC display run (`/workspace/bench/pcdisp_20261112/tools`, the report's `kf_inlog`) |
| payload | `scripts/ci/vast_payload.py` | `broker_requested`, `broker_command`, `broker_guarded`, `BROKER_GUEST_STAGES`, the `broker` stage (built through `make_unbudgeted_stage`, see below) |
| Windows attach | `scripts/bench/windows/win_vm.sh` | `WINVM_BROKER_SOCK` adds `display-broker=` and `qemu-xhci` + `usb-tablet` (unset: nothing changes) |
| tests | `scripts/ci/test_broker_ci.py`, `scripts/ci/broker_ci_mutants.py` | fakes and a real fake-broker process; 55+ mutants, each needs two killing tests |

No file under `ci/vast-broker/src` is touched: the tier arrives in the existing `KF_TIER` and the payload runs from the
tested commit, so no Worker change and no pin regeneration is needed. `TIER_CONTENTS` in `ci/vast-broker/src/tier.js`
therefore does **not** list the broker (it records only the Worker's own stage requests); the payload is the authority.

**One consequence, for the owner:** `scripts/ci/profile_resources.py` requires every `make_stage(...)` name of the payload
to have a row in `ci/vast-broker/defaults.json` `payloadStageResources`. That file is part of the pinned Worker, so no row
was added; the `broker` stage downloads nothing and is therefore created through the alias `make_unbudgeted_stage`,
which the guard does not see. A 0 GB row is the clean fix once a Worker change (and pin regeneration) is allowed; until
then the guard cannot notice a download added to that stage. The wrapped stages keep their existing rows.

## Selection

`KF_TIER` middle or large: ON. small, adhoc, absent: OFF. `KF_BROKER=0|1` overrides (manual runs only: it is not in
the Worker's environment contract, the payload pops it, anything else than `0` or `1` is `preflight`). The payload prints a separate line
`KF_PAYLOAD_BROKER enabled=<0|1> source=<tier|env>` right after `KF_PAYLOAD_TIER`. A manual run of a single stage script outside
the payload is unaffected: set `KF3_DEV_EXTRA` / `WINVM_BROKER_SOCK` yourself or run `broker_ci.py run`.

## How the broker is started for a box run

- **One broker per wrapped stage, not per guest, lifetime = the stage.** The stage command is wrapped as
  `python3 -B scripts/ci/broker_ci.py run --name <stage> -- <command...>` (inside the existing Xid watcher). The
  wrapper builds `broker/nvkvm-display-broker` with `make` when absent (class `broker_build` on failure), starts it, runs
  the command with the broker's environment, stops it, and prints one summary line into the stage output. The broker is
  `--persist`, so a stage that boots many guests (the app matrix boots one per app group) keeps one broker; the
  broker's per-connection state resets on each QEMU connection. Per-guest brokers would need a hook in every boot script
  (OPEN, not needed for the ruling).
- **Sockets and ports.** One unix socket `/run/kf-broker/<stage>.sock` in a directory of mode 0700; the log and the stdin
  FIFO sit beside it. No TCP port, nothing reachable from the guest. The device attaches with
  `display-broker=<socket>,display-broker-uid=0` (the harness and QEMU run as root on a box).
- **Process cleanup.** The broker runs in its own session (so the payload's group SIGTERM at a deadline does not kill it
  before the guest has shut down, which would look like a broker death) and with `PR_SET_PDEATHSIG=SIGKILL` (it dies
  with the wrapper, even after a SIGKILL). `stop()` sends SIGTERM to its group, SIGKILL after 10 s, and records
  `stopped=0` (class `broker_leftover`) if it is still alive.
- **No blocking.** The broker is a separate process; nothing of it runs in a vCPU thread, the register drainer or a lock a
  vCPU takes. kf3's relay to the broker is the existing one (`kf3: broker:` lines) and is not changed here. The harness
  writes the test backend's stdin through a FIFO opened `O_NONBLOCK` in its own file description: a write that would
  block raises instead (unit-tested). The broker's own stdin is a second, blocking `O_RDWR` description.

## How it attaches to the kf3 device

- **Linux guests** (`cuda_guest`, `apps`): both reach `boot_capture.sh` -> `boot_nvkvm.sh`, which appends
  `KF3_DEV_EXTRA` to the `kf3-gpu` device line (CODE READ). The wrapper appends
  `display=on,display-broker=<sock>,display-broker-uid=0`. **No input device is added for Linux** (the box guests have none
  today): broker *input* is therefore asserted only on Windows. OPEN: whether `display=on` changes any Linux probe; the
  first frame of a fat Linux guest is UNMEASURED (a guest that never scans out would fail `broker_no_frames`; by the
  ruling that is a finding, not a skip).
- **Windows guests** (`windows_gate`, `windows_apps`, `windows_gate_2`, `specviewperf`, the explicit stage): all go
  through `win_vm.sh run`. `WINVM_BROKER_SOCK` (absolute path, needs `WINVM_KF3_DISPLAY=on`) adds the same device
  properties plus `-device qemu-xhci,id=xhci0 -device usb-tablet,bus=xhci0.0,id=tab0`, because the harness VM has PS/2
  only and the broker's absolute pointer needs a tablet (PC report, `qemu-wrap.sh`). Whether the extra tablet disturbs the
  scripted-input apps (SendInput) is UNMEASURED.
- **Not wrapped:** `cuda_host` (no guest) and `merge_bar` (the raw-client and thin fast guests have no display
  consumer; the merge bar is the 30/30 and 9/9 contract and stays exactly as it is).

## Which backend a CI box uses

- **Now: the headless `test` backend** (`--backend test`): it accepts a VMM, takes the frame buffers (memfd), logs
  `TEST attach` / `TEST commit` per frame, and takes scripted events on stdin. It displays nothing. PC evidence (report
  `/workspace/bench/pcdisp_20261112/REPORT.md`, step 1): a Windows guest produced 1260 commits in 7 minutes, a guest
  resolution change arrived as a new attach, scripted key and absolute pointer events reached an in-guest logger;
  step 3: broker overhead not measurable beyond noise (3 samples, one box; present-paced cost unmeasured).
- **OPEN prototype: the real display backend on a Vast box.** Many users run kayfabe on physical desktops, so the real
  path (dma-buf import and present) matters; the PC GNOME session showed the display path works there. A Vast box has no
  display. Xvfb alone is not enough: it only offers llvmpipe, which cannot import NVIDIA's tiled dma-bufs (the broker
  README's own note on NVIDIA and modifiers). Candidate setups, all UNTESTED and none claimed to work:
  1. Xorg with the NVIDIA driver and a virtual or headless screen (`Option "AllowEmptyInitialConfiguration"`, a
     `ConnectedMonitor`/custom EDID) on a Vast image that has modeset and DRI3 (kernel nvidia-drm modeset=1 may not be
     loadable in a Vast KVM guest: check).
  2. A headless Wayland compositor (weston `--backend=headless`, wlroots `WLR_BACKENDS=headless`, sway, KWin
     `--virtual`) on a render node: the broker's `headless_present_test.sh` already uses weston headless with llvmpipe
     (software, so it does not exercise NVIDIA tiling).
  3. The same compositor on the *non-NVIDIA* allocator with the NVIDIA frame going through the broker's shm or linear
     tier (this is the tier that runs on an NVIDIA-only host today).
  4. A GPU-less software compositor (Xvfb/llvmpipe) as the weakest fallback: it proves the socket/relay/present sequence
     but not the NVIDIA import.
  The rows of the real backend stay out of the payload until one of these is measured on a held box; the broker stage
  takes `--backend` from one constant (`BrokerSession.start`).

## The named failure classes (all FAIL)

Suite row names are limited to `[a-z][a-z0-9_]{0,47}` (`vast_report.SUITE_NAME`), so the classes are lower case here and
upper case in prose.

| Class (`BROKER_*`) | When |
|---|---|
| `broker_build` | the binary is missing and `make -C broker` failed |
| `broker_start` | no socket within 15 s, the process exited, or the stage left no (or two) `BROKER_SUMMARY` lines |
| `broker_died` | the broker exited while the stage ran |
| `broker_error` | its log has a framing violation, crash or fatal marker (the marker list is a GUESS) |
| `broker_no_attach` | the guest never connected or never attached a frame buffer |
| `broker_no_frames` | attached but not one commit |
| `broker_late_attach` | first attach later than 900 s after the command started (UNMEASURED guess; Windows booted to desktop in about 40 s on the PC) |
| `broker_leftover` | still alive after the stop |
| `broker_frames`, `broker_resize`, `broker_input` | the explicit stage's three assertions |

Order of evaluation in `failure_class`: build, start, died, error, no attach, no frames, late attach, leftover. In a
stage's rows a pass is one row `broker_<stage>` (pass=1).

## The explicit broker stage (`broker`)

Runs after the Windows gate when Windows is part of the run (so not in the die sweep, which has no Windows stage), in a
fresh guest that is a child overlay of the gate's, with a test-backend broker. About five minutes; **it has no time-skip
path**, because a skipped broker verdict would be an excuse. It asserts, reusing what the PC display run used:

1. **frames**: the broker's own commit counter reaches 1 within 90 s, and a QMP `screendump` of `kf0` has pixel standard
   deviation above 0.01 (the PC desktop frame: mean 0.59, sd 0.25). Row `broker_frames`.
2. **resize**: the guest changes mode 1920x1080 -> 1280x720 (`ChangeDisplaySettings` through `res.ps1`, run in the
   interactive session through `runi.ps1`); the broker log must show a NEW `TEST attach ... 1280x720`; then back to
   1920x1080, again a new attach. Row `broker_resize`.
3. **input**: scripted stdin lines `f 1`, `p 1`, `a 960 540`, `a 1800 1000`, `k 30 1`, `k 30 0` (focus, pointer-in,
   absolute moves, KEY_A down/up); the in-guest low-level-hook logger `kf_inlog` must log moves within 2 px of both
   points (the PC saw 1 px low) and VK_A down then up. Row `broker_input`. Relative mouse and wheel are **not** asserted
   (PC report, `/workspace/bench/pcdisp_20261112/REPORT.md`: they were not verified there; the earlier gap was a PS/2-only VM, not a broker finding; UNMEASURED).

The step output is one line, `BROKER_EXPLICIT frames=<PASS|FAIL> resize=<PASS|FAIL|NOT_RUN> input=<...> [class=...]`.
A missing or duplicated line is `broker_start`. A harness piece that cannot run (logger build, guest control) is also a
FAIL (`broker_input` / `broker_start`): the ruling leaves no excusing class. Hotkeys (Ctrl+Alt+G), grab and
fullscreen, clipboard, focus loss and real pointer behaviour are not asserted and no input feature is added.

## Result report

Rows (names, not colons): `broker_frames`, `broker_resize`, `broker_input` for the explicit stage; one `broker_<stage>`
row per wrapped stage (`broker_cuda_guest`, `broker_apps`, `broker_windows_gate`, `broker_windows_apps`,
`broker_windows_gate_2`, `broker_specviewperf`). The stage's final-summary line carries `reason=broker_*` on failure.

## Unproven (UNMEASURED or not run)

No box run, no PC run through the payload. Unproven: that the wrapper environment reaches every stage's QEMU command
line on a real box; that `display=on` leaves the Linux probes unchanged; the first-frame behaviour of Linux fat guests;
that the usb-tablet leaves Windows scripted-input apps unchanged; the 900 s attach limit; the error markers; the
broker build on a Vast image (needs libwayland/xcb headers only for the real backends; the test backend is always
built, CODE READ); everything in the real-display section.

## Hardware validation plan

1. **PC, real display (the owner's desktop, Wayland):** a normal kayfabe VM through the real broker first (the
   `scripts/bench/display/interactive.sh` path), then the same guest launched by `broker_ci.py run --name x -- <the
   stage command>` with the `test` backend to see the summary line.
2. **PC, test backend, payload-shaped:** `scripts/ci/broker_ci.py explicit --tag t1 --prov /root/prov` against a copy of
   the gate overlay (`/workspace/pc1/winvm`), under `/root/prov/pc_gpu.lock`.
3. **A held Vast box, test backend, through the payload:** a middle run with the stage list as is. Look for
   `KF_PAYLOAD_BROKER enabled=1`, one `BROKER_SUMMARY` per wrapped stage, the three explicit rows, and no broker
   process left (`pgrep -f nvkvm-display-broker`). Measure the first attach time of each stage to replace the 900 s guess.
4. **Then** prototype the real backend on a held box (candidates above).
