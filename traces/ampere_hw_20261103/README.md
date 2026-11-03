# Ampere: the Linux bar and the first Windows gate on a GA10x die (2026-11-03)

STATUS: LIVE, 2026-11-03. Pre-registration first (written and pushed BEFORE any measurement or run on the box);
results are appended under "RESULTS". MEASURED = a line in a committed log, named with its file; INFERRED =
reasoning; COMPUTED = a GPU-free computation from the committed code.

## What is booted

Branch `claude/ampere-hw-20261103` = `origin/claude/turing-boot-20261103` (PR #243: `next` with the Blackwell merge
#239, the Windows gate and its fixes) plus whatever this lane adds (listed under RESULTS with its revision). Flags:
`scripts/bench/windows/kf3_flags.sample.env` unchanged; no `KF3_DIAG_*` variable. Host driver: provisioning's
default, 580.159.04.

COMPUTED (`crates/kf-disp/data/caps_measured.tsv` at the base): there is NO tier-3 row for any Ampere die
(architecture 0x170). At the base revision kf3 therefore serves an Ampere die the clean T1 page and logs
`NO MEASURED caps word 0x00c ... a Windows guest's display may not come up` (`Derived::unmeasured` non-empty). The
Turing storm (traces/turing_boot_20261103/) and the GB203 bwhw1 run show what that page costs on Windows.

## Hypotheses and falsifiers (pre-registered)

- **H-ga-00c-measurable**: the Ampere die's caps word `0x00c` reads back non-zero under the stock host driver with
  `tools/read_display_caps_page.py` as root (read-only BAR0, PROT_READ). No value is predicted (every measured die
  so far has 0x100 or 0x102). FALSIFIER: the tool exits non-zero, or the page is empty/all-ones, or a new host Xid
  appears around the read. The value goes into `caps_measured.tsv` only for driver branch 580 (the host it was read
  on, 580.159.04, and the guest's 580.88), with a failing-first GPU-free test that the derived page for this die has
  `unmeasured` empty.
- **H-linux-ampere**: on the exact revision with that row, the Linux bar holds on this die:
  `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS|FALLBACK`, `KF3_RC=0`, `BARE_SUITE_PASS=30 ... ARMS=30`,
  `FAST_SUITE_PASS=30 ... ARMS=30`, `BIRTH_CENSUS_OK`. FALSIFIER: any line missing or different. Known exception
  pre-stated (another lane's walk-latency regression in `next`): if the ONLY miss is `ce-client-guest-ram`
  `verdict=TIMEOUT`, that arm is re-run alone at a 600 s budget and recorded; the 180 s bar is then NOT met on this
  revision and is reported as such.
- **H-windows-ampere**: with the derived page (`caps page derived — N words: ... (0 unmeasured)`), the Windows LTSC
  gate passes on this die: `WINDOWS_GATE_PASS ... resets=0`, a window PUT after the modeset (`the console shows head
  0 ... [window 0`), exactly one `Running -> Suspending` (the graceful stop), no `USER_RING_SCAN` rejection in the
  kf3 log (the Ampere `0xC56F` user rings are admitted), host Xid delta 0. Target n>=2 (fresh overlays).
  - FALSIFIER: `WINDOWS_GATE_FAIL`, any reset counter > 0 since the enable, more than one `Running -> Suspending`,
    an `unmeasured` count > 0, no window PUT after the modeset, or a `USER_RING_SCAN` rejection.
  - On a FAIL: state at failure (the gate's own `state_at_failure_*`), then the first arm is the display path (as
    on GB203: did the core channel get an `UPDATE`?), then the user-ring path. Max 3 negative boots per wall.
  - A `WINDOWS_GATE_RETRY` is not a verdict on the guest: fix the script or re-run on a fresh box.

## Box (own id only)

| box | id | GPU | offer | where | created |
|---|---|---|---|---|---|
| A | 58969637 | RTX 3090 (GA102), 32 vCPU, ~250 GB RAM, 160 GB disk requested | 46019435 | United Kingdom | 2026-11-03T13:58Z |

## RESULTS

### Box A (MEASURED 2026-11-03): GA102 RTX 3090 `10de:2204`, 30 vCPU, 98 GB RAM, 155 GB root, host 580.159.04

`boxA/prov.log`: `BOX_RC=0 DRIVER_RC=0 TREE_RC=0 LADDER_RC=0 FG_RC=0 KF3_RC=0`, `READY driver=580.159.04` (14 min).

**H-ga-00c-measurable: not falsified.** `caps/ga102_caps_host580.txt` (read 14:14Z, host driver up, no guest):
`DECISIVE 0x00c 0x00000100`, `EVENCAPA` of windows 0/2/4/6 = `0x01df21d0` (bit 19, `SCLR_PRESENT`, set), 95
non-zero words, tool rc=0, host Xid 0 before and 0 after (`caps/read_context.txt`). Row added for branch 580
(`crates/kf-disp/data/caps_measured.tsv`, commit `57e8fed2`); test `crates/kf-disp/tests/ampere_caps.rs` red before
the row (`caps/test_red.txt`: `GA102 has a measured 0x00c row ... [(12, ...)]`), green after (`caps/test_green.txt`);
`cargo test -p kf-disp` all green. GA103/GA104/GA106/GA107 still have no row and keep the clean page and the notice.

**H-linux-ampere: not falsified** (`merge_check.sh claude/ampere-hw-20261103 am1`, rev `006d845a`, `boxA/am1.log`):

```
V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS
KF3_RC=0
BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30
FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30
BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12 all PRIVILEGED_CHANNEL=0 privilege=USER
```

The thin-guest suite took 870 s of arm time (sum of `secs=`, `boxA/am1_suite.out`); `ce-client-guest-ram` passed in
119 s (`client_ms=98070`) inside the 180 s budget, so the walk-latency exception pre-stated above did not arise on
this box. (INFERRED: a 30-vCPU box hides most of it; no comparison against 9297ffdd9 was run here.)

### Box A, Windows gate run am1 (MEASURED 2026-11-03, rev `006d845a`, the `KF_MERGE_WINDOWS=1` stage of the same merge check)

The first Windows gate result on any Ampere die. `boxA/am1_windows.log`:

```
WINDOWS_GATE_PASS rev=006d845a gpu="NVIDIA GeForce RTX 3090" idle_s=300 hostref=green/green secs=3661 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=3 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-03T15:28:07Z
WINDOWS_GATE_EXIT rc=0 2026-11-03T15:42:50+00:00
```

`WINDOWS_GATE_TIMING total=3662s ... install=1852s ... nv_install=444s ... idle=329s ...` (install SMP 16, `cache=unsafe`).
Against the pre-registered falsifier (files under `boxA/am1_win/`; UUIDs scrubbed, so `MANIFEST.sha256` no longer
matches the scrubbed files):

- kf3 log `gate_am1_b2_qemu.log.gz`: `caps page derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0
  unmeasured)` — the GA102 row served, no notice; `+253541 ms the console shows head 0 1920x1080: [window 0 ...]`
  (a window PUT after the modeset).
- User rings: 0 `USER_RING_SCAN` lines; the guest's `0xC56F` channels were allocated (`RmAlloc ... class=0xc56f
  result=0x0`) and the GR tier admitted host channels `privilege=USER`; no `PRIVILEGED_CHANNEL=` line other than 0.
- `gsp_signals.txt`: `running_to_suspending=1` (the stop), `unloading_guest_driver=1`, `host_xid_in_boots=0`.
- `tdr_counts.txt`: `display4101=0 nvlddmkm153=0 kernelpower41=0 eventlog6008=0 bugcheck1001=0
  livekernelevent_other=0 livekernelevent193=3 ... total=0`; one WATCHDOG dump 15:29:03Z, +56 s after the enable,
  before the probes (the same reported-not-gated LiveKernelEvent 193 as on AD102 and TU106).
- Host reference green before and after (`hostref_after.txt`: `HOSTREF_CUP8 PASS`, `HOSTREF_XID_DELTA 0`; the
  count of 10 is the suites' intentional Xid controls). Host BAR1 is 256 MiB on this box; kf3 ran `bar1-size=128 MiB`,
  `fb-mb=8192`.
- The one `kf3: device reset #1 ... before the vCPUs resume` is the platform reset at boot, as in TU106 tbA2.
- No QEMU and no `win_vm.sh run` loop after the gate (`pgrep`, empty, before am2 was launched).

H-windows-ampere: not falsified (n=1).

⚠ Evidence hygiene: commit `edd35d3a` carried this board's GPU UUID in `boxA/am1_host.log` (the bare-metal suite's
`nvidia-smi` line). `d4347d7a` scrubs it at HEAD (`no_gpu_uuids.py`: new=0); the repository refuses
force-pushes, so the UUID stays in that one commit's history (for the release history sweep).
