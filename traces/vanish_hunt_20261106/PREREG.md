# Vanishing Worker boxes: pre-registration, 2026-11-06

STATUS: RESEARCH, 2026-11-06. Written BEFORE any rental of this hunt (agent `vanish`). The results go in
`README.md` beside this file; this file is not edited after the first rental except for marked addenda.

## The question

Worker-rented Vast boxes end mid-run with no result: gpu-vast run 42228490212 attempts 4 (RTX 3070, machine
56506, host 79466, lost during `merge_bar`) and 6 (RTX 4090, machine 17049, host 96607, lost ~148 s into
`cuda_guest`), and run 42039008874 attempt 1 (RTX 3060, machine 17023, host 79466, lost during `merge_bar`).
Who ended them: the box (kayfabe or the box kernel), the host, Vast, or the Worker?

## What the logs already say (read before renting; the runner logs are not committed, the lines are quoted)

- Attempt 6 (`/tmp/gsr-b3f.log`, coordinator copy): the Worker's last line is `[Worker] provider=unknown
  phase=TESTING`; the runner's diagnostics read `state=NONE verdict=FAIL`, `rent to last box POST 2512s, rent to
  end 2596s`. Attempt 4 (`/tmp/gsr-b3d.log`): `[Worker] provider=unknown phase=TESTING`, `rent to last box POST
  2218s, rent to end 2222s` (the box POSTed 4 s before the Worker ended the run). Run 42039008874 attempt 1
  (`/tmp/gsr-batch-job.log`) and its re-run (`gsr-batch-job2.log`, lost right after `cuda_guest` started): the last
  Worker line is `[Worker] provider=created phase=TESTING`.
- INFERRED from `ci/vast-broker/src/index.js` (bundle tree `ecf8c807`): `provider=<x>` with `<x>` = `unknown` is
  printed when Vast's `actual_status` is not one of running/created/loading/starting/stopped/exited (for example
  null or a value the Worker does not know). After the first box report, ANY `actual_status` other than `running`
  makes `update()` call `close(record, 'ENDED')` (verdict FAIL), which destroys the instance at once. So in all
  four losses the Worker itself destroyed the instance after one non-`running` reading. What is NOT known: whether
  the box was already dead (status change = consequence) or alive (status change = provider transient).

## Every way the Worker ends a rental (src/index.js at ecf8c807) and the line it leaves

| path | trigger | line in the run log | verdict |
|---|---|---|---|
| update() HARD_TIMEOUT | now >= started + ttl (`hardMinutes` 300; profile caps may shorten) | `[Worker] destroy reason=HARD_TIMEOUT` | FAIL |
| update() RESULT | result grace over (pass 5 / fail 10 min) | `[Worker] destroy reason=RESULT` | the result's |
| update() BOOT_TIMEOUT | no first report by `bootMinutes` 75 | `[Worker] destroy reason=BOOT_TIMEOUT` (+ `host excluded after frozen KVM boot signature`) | UNAVAILABLE / PROVIDER_FAULT |
| update() HEARTBEAT_TIMEOUT | 15 min without a box POST | `[Worker] destroy reason=HEARTBEAT_TIMEOUT` | FAIL |
| update() inspect throws | provider API error | `[Worker] provider_error` (closed UNAVAILABLE) | UNAVAILABLE |
| update() instance null | Vast no longer lists it | `[Worker] provider_absent` (no destroy; GONE) | UNAVAILABLE |
| update() not running | `actual_status` != running after the first report | `[Worker] provider=<state> phase=TESTING`, then NO destroy line | FAIL |
| update() RUN_GONE | GitHub run completed or missing | none | FAIL |
| checkContract | price/inet/budget re-check fails | none (CONTRACT_PRICE / CONTRACT_BUDGET) | UNAVAILABLE / PROVIDER_FAULT |
| report() liveness | on a box POST (>= 3 s since the last check), `actual_status` != running | none (close UNAVAILABLE, POST gets 401) | UNAVAILABLE |
| report() DEADLINE / TTL / LIMIT | expired callback, report expiry/hard deadline, report count/bytes cap | `destroy reason=DEADLINE callback`; TTL/LIMIT none | FAIL |
| report() PROVIDER_FAULT | result carries a provider fault | `[Worker] provider fault: machine ...` | PROVIDER_FAULT |
| kill / stop / stop-all / batch stop / SUPERSEDED | box `kf-box-kill`, runner or owner stop, newer run on the branch | none | FAIL |

The observed losses match exactly one row: `update() not running` (a `provider=` line in TESTING, verdict FAIL, no
destroy line). Attempt 4's 4 s gap between the last POST and the end also fits it (a runner status poll).

## Hypotheses, each with its falsifier (stated before the run)

- **H-BOX-OOM/PANIC** — the box VM's kernel dies (OOM, panic, watchdog, reboot) in the heavy stages, and Vast then
  reports a non-running status. *Falsified if* on our direct rental the box's out-of-band stream shows no OOM/panic
  and the box keeps answering ssh through every stage, or if Vast reports non-running while ssh still answers and
  uptime does not reset. *Supported if* the stream ends with OOM-killer/panic/hung-task lines and ssh stops before
  the status changes.
- **H-GPU-FAULT** — the deliberate fault arms (Xid 31 etc.) wedge the passthrough GPU or PCIe link (AER) and take
  the nested VM down on some hosts. *Falsified if* the box survives `merge_bar` and `cuda_guest` with the same Xid
  set and no AER/`fallen off the bus`/Xid 79 lines. *Supported if* AER, Xid 79/119/154 or `GPU has fallen off the
  bus` appear in the last streamed lines before death.
- **H-VAST-HOST** — the host or Vast stops/preempts/reboots the instance (or the host loses its Vast daemon link).
  *Supported if* `actual_status`/`status_msg` turn non-running and the box is unreachable with no guest-side
  cause in its last lines, or the box comes back with a reset uptime. *Falsified (for this rental) if* status stays
  `running` throughout.
- **H-WORKER (transient status)** — Vast's per-instance API reports a transient non-`running` `actual_status`
  (null/`created`/other) while the box is alive, and the Worker's zero-tolerance close destroys a live box. *Supported
  (MEASURED) if* our poller of the same endpoint (`GET /api/v0/instances/<id>/`, every 10 s) records any non-running
  `actual_status` while ssh answers and the payload keeps progressing. *Falsified (for this rental) if* every
  poll reads `running` while the box is alive. A direct rental cannot falsify the Worker's other paths; the table above
  does that from the code.
- **H-NEW-CODE** — something in `ecf8c807` (vs `49da9f9e`/`88ffc681c`) kills boxes. *Prior:* low; no crate changes
  between the passing batch-2 tree and this tree (coordinator's statement, to be checked with `git diff --stat`).
  Tested only if budget allows: same stages, older tree, same host.

## Capture plan (direct rental, no Worker, no secrets on the box)

1. Rent one 1-GPU RTX 4090 on machine 17049 / host 96607 (offered at 0.441 $/h when this was written), KVM image
   `docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16`, `--disk 120`, label `kf-vanish-20261106`; the
   `instance_api_key` is filtered out of the create output.
2. Coordinator poller (no ssh): every 10 s, `GET /api/v0/instances/<id>/` -> `actual_status`, `intended_status`,
   `cur_state`, `next_state`, `status_msg`, uptime fields, to `vast_status.tsv` (IPs, ports, keys dropped).
3. Coordinator liveness probe: every 10 s `ssh -o ConnectTimeout=8 <box> 'cat /proc/uptime'` -> `ssh_probe.tsv`.
4. Box streams over long-lived ssh sessions, written on the coordinator: `dmesg -wT` (kernel incl. OOM, Xid, AER,
   hung tasks), a 5 s sampler (`free -m`, `/proc/pressure/{memory,io,cpu}`, loadavg, qemu RSS from `ps`), and the
   payload log `tail -F`. The box also keeps its own copies under `/root/vanish/` (persistent journal enabled) for
   a post-reboot read.
5. The payload runs as the Worker's does: `python3 -I scripts/ci/vast_payload.py <sha> <nonce>` from a checkout of
   `ecf8c807` at `/workspace/kf-ci-src`, `KF_MERGE_WINDOWS=0` (the losses happened before the Windows stage),
   soft deadline = rent + 2.5 h, no setup cache (no Worker blob route; provision downloads from the network).
6. If the instance exits: it is NOT destroyed until `vastai show instance`, `vastai logs` (serial console) and the
   poller have recorded its status; then destroyed by exact id with verified absence.
7. If the box does not die: the same on a second host (79466 only if offered) if budget allows.
