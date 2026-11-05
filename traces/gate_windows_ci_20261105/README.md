# First Windows gate PASS through the Vast CI Worker, 2026-11-05

STATUS: LIVE, 2026-11-05. One run of the `gpu-vast` check (profile `check`) on a real Vast box, through the deployed Worker, ended with `verdict=PASS` and the `windows_gate` stage as its last of five. Labels: MEASURED means a line in a file in this directory, and the file is named. INFERRED means reasoning from such lines or from the code. Box text is untrusted data (`[untrusted]` lines, kept as the runner printed them).

Files (both were collected by the coordinator from the job logs of GitHub Actions run 41936606326; the runner had already redacted them, and GitHub's own `***` masks are in them):

- `run41936606326_broker.log`: the `gpu-vast` broker job (23:49:02Z on 2026-11-04 to about 01:21Z on 2026-11-05).
- `run41936606326_gate_status.log`: the job that posts the `gpu-vast` commit status (01:20:31Z).

## Identity of the run

| item | value | source |
|---|---|---|
| Actions run | 41936606326 | coordinator (the log header does not print the run id) |
| tested tree | `next` at d74f6063d (`TARGET_SHA` d74f6063d2cadb303b1f4f374149e435f632d51c; the box reports `d74f6063 Merge pull request #254`) | MEASURED, `run41936606326_broker.log`, env block and the box's `git log` line |
| trigger | PR #255 against master, since closed (the path of #249 in `../gate_windows_ci_20261104/README.md`) | coordinator; the log does not name the PR |
| trusted BASE (runner side) | db796290b945857c7a95b1a7442e447f5f2cc3ce | MEASURED, `run41936606326_broker.log`, line "Trusted BASE" |
| Worker bundle | the runner expected SHA256 2828bf41ed25...0bb0 (the master pin) and accepted the reply; the box printed `KF_PAYLOAD_TIER windows=1 source=default` (this Worker sends no tier) | MEASURED, same file. INFERRED: the deployed bundle was 2828bf41, because run 1 of the 1104 series showed the runner rejecting a mismatching bundle after the rental. |
| selection | `profile=check die=GA102 reason=DEFAULT_ORDER ignored=0` | MEASURED, same file |
| rental | machine 17545, host 63709, Vast instance 59178654, RTX 3090 (GA102), 0.466667 USD/h (offer table showed 0.482292 effective, 1.454653 estimated total) | MEASURED, same file |
| host driver | 580.159.04 | MEASURED, same file (`DRIVER_RC=0` line and `GATE_HOSTREF`) |
| result | broker state GRACE, verified 1, verdict PASS, commit status `gpu-vast` success | MEASURED, `run41936606326_gate_status.log` env block (`BROKER_STATE: GRACE`, `BROKER_VERIFIED: 1`, `BROKER_VERDICT: PASS`) |

## Stages

MEASURED (run 41936606326, rev d74f6063, 2026-11-05), `run41936606326_broker.log`, the `KF_FINAL_SUMMARY` block and the `KF_PAYLOAD_STAGE_END` lines. Minutes are computed from `duration_ms`.

| stage | rc | duration_ms | minutes | result |
|---|---|---|---|---|
| provision | 0 | 435715 | 7.3 | PASS |
| cuda_host | 0 | 13302 | 0.2 | PASS |
| merge_bar | 6 | 988795 | 16.5 | PASS |
| cuda_guest | 0 | 239021 | 4.0 | PASS |
| windows_gate | 0 | 3692199 | 61.5 | PASS |
| sum of the five | | 5369032 | 89.5 | |

Suite rows of the same summary: `provision` 1, `cuda_host` 4, `gates` 9, `kf3_build` 1, `raw_suite` 30, `thin_suite` 30, `birth_census` 1, `cuda_fat` 4, `windows_gate` 1, all `fail=0 skip=0`. The setup cache restored in 31.4 s (`KF_SETUP_CACHE_END`).

The Windows gate lines, verbatim from the log:

```
WINDOWS_GATE_PASS rev=d74f6063 gpu="NVIDIA GeForce RTX 3090" idle_s=300 hostref=green/green secs=3688 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=0 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-05T01:09:23Z
WINDOWS_GATE_TIMING total=3688s preflight=8s hostref_before=6s prereqs=9s kf3_binary=0s install=2627s probes_build=3s boot1=76s check_kf3=28s nv_install=159s desktop_prep=12s reboot=91s desktop=18s enable=23s smi_probes=59s idle=315s post_idle=72s collect_stop=156s
```

Also MEASURED in the same file: `GATE_NOTE second QEMU started after 20s` and `GATE_STAGE reboot EXIT ok 91s` (the stage that failed on GA106 in run 3 of the 1104 series), `GATE_BOOT ... os='Microsoft Windows 11 Enterprise LTSC Evaluation' build=26100`, `GATE_DESKTOP user='KFWIN\kf' dwm=1 explorer=1`, `GATE_TDR ... total=0`, `WIN_CHECK_DONE fail=0`, `GATE_HOSTREF after green cup8=PASS ... xid_delta=0`, and the probes `cup8`, `cup3`, `d3d11`, `d3d12` all `rc=0`.

## What `merge_bar rc=6` means

MEASURED (run 41936606326, rev d74f6063, 2026-11-05): the summary prints `stage name=merge_bar rc=6 duration_ms=988795 result=PASS`. The rc is the exit status of `scripts/bench/host_xid_watch.sh`, which exits 6 when the host kernel log has new Xid lines across the wrapped command (`exit 6` after the line "NEW HOST Xid ACROSS THE COMMAND", `scripts/bench/host_xid_watch.sh`). It is not a failure by itself. In `scripts/ci/vast_payload.py`, `stage_grade` takes rc 0 and rc 6 to the grader; `watch_delta` demands that the Xid delta is non-empty and equals `after - before`, and the stage passes only if the delta equals, as a multiset, the lines in the file named by `KF_PAYLOAD_XID_ALLOWED` (written by the guest-arm approval in `native_xid_guard`). Any other delta, an unreadable log, or an rc other than 0 and 6 is an `EvidenceError("host_xid")`. The `result=PASS` therefore says that the payload's grader accepted the delta.

What this directory does not show: the Xid lines themselves. The runner's bounded log dropped about 1.06 MB from the middle (`[1056298 bytes dropped from the middle]`), and that span holds the `merge_bar` and `cuda_guest` output. INFERRED from the lines that remain: `provision` and `cuda_host` ended with `xid_before=0 xid_after=0`, and the `windows_gate` watcher started with 10 Xid lines already in the ring buffer (`xid_after=10`, "10 before, 10 after"), so the 10 lines appeared during `merge_bar` (a `cuda_guest` rc of 0 requires a clean delta in `watch_delta`). Which Xids they were is NOT KNOWN from this evidence, and the Xids are not explained here.

## Caveats

- n = 1: one run, one die (GA102, RTX 3090), one box. It is not a statement about other families or about repeat rate. The earlier attempts of the 1104 series had two Linux guest arms fail intermittently on a GA106 box; this run shows nothing about whether those flakes are gone.
- This was a cold install: `install=once`, 2627 s of the 3688 s gate. Run 3 of the 1104 series had a 3197 s install on GA106; the two are different boxes and dies.
- The old Worker (2828bf41) with its 180-minute limit ran this. The payload used 89.5 minutes of it. A slower box or a longer install has less margin, and the new Worker with the 300-minute limit was not deployed in this run (INFERRED from `KF_PAYLOAD_TIER ... source=default`).
- The trigger-PR path was used because `gpu-vast.yml` admits a rental only for a PR against master or a push to master; that is the same documented path as #249. The trust model is unchanged.
- `GATE_TIMELINE` prints `first_dump=none dump_minus_enable=35437s`: with no dump the field is not a measurement and it is not interpreted here.
- The teardown of instance 59178654 is not in these logs (the status job reports the state GRACE). This lane rented nothing and did not check Vast, so the absence of the instance is NOT SHOWN here. Cost INFERRED: the payload ran 89.5 min at 0.466667 USD/h, about 0.70 USD, plus setup and the grace window, which are not in the logs.
- The gate's evidence archive (`..._windows.tgz`, sha256 prefix 154b577caa862b1a) was on the box and is not in this directory.
