# Vanishing Worker boxes: the hunt, 2026-11-06

STATUS: RESEARCH, 2026-11-06. Agent `vanish`, coordinator host, direct Vast rentals (no Worker), tree `ecf8c807`
(next, the tree gpu-vast run 42228490212 tested). Hypotheses and falsifiers were written first:
[PREREG.md](PREREG.md). MEASURED = a line in a file in this directory (named), or in the Worker source at
`ecf8c807`; INFERRED = reasoning, marked as such.

## Answer in four lines

1. **The Worker destroys the box itself.** All four losses in TESTING end on the Worker line
   `[Worker] provider=unknown phase=TESTING` (run 42228490212 attempts 4 and 6) or `provider=created phase=TESTING`
   (run 42039008874, both 3060 jobs on host 79466). At `ecf8c807` that line exists only in `update()`; when the state
   is not `running` after the first box report, the next statement is `close(record, 'ENDED')`, which destroys the
   instance on **one** reading (`ci/vast-broker/src/index.js` lines 402-451). The report path does the same with no
   log line at all (lines 523-537). MEASURED from the code and the runner logs. The Worker bundle that ran
   (`ae7de5c6`, deployed about 07:00Z) was not rebuilt here. Its trusted base `88ffc681c` and `ecf8c807` differ in
   `src/index.js`, but no changed line touches `actual_status`, `providerState`, `pendingInstance` or `close(`
   (`git diff 88ffc681c ecf8c807 -- ci/vast-broker/src/index.js`).
2. **Vast's per-instance status flaps.** On host 96607 / machine 17049 (attempt 6's machine), `GET
   /api/v0/instances/<id>/`, the endpoint the Worker reads, returned `actual_status: "offline"` in 16 of 174
   samples taken every 10 s, while `intended_status` and `cur_state` stayed `running`. The flaps were single 10 s samples
   between `created` readings, plus one stretch of about 90 s (`i59440972_h96607_m17049_no_boot/vast_status.tsv`).
   `offline` is not in the Worker's list, so the Worker prints it as `unknown`, the exact word in attempts 4 and 6.
   MEASURED, during BOOTING only: the VM never finished booting, so no flap during TESTING was observed.
3. **kayfabe's payload does not kill the box on the machines tested.** The full Worker payload at `ecf8c807`
   (provision, cuda_host, merge_bar, cuda_guest; Windows off) PASSED on attempt 4's own machine (RTX 3070, machine
   56506, host 79466) and on an 8-way RTX 4090 host (machine 55267, host 33035). Both runs had no OOM, panic or
   reboot, PSI memory at 0, at least 44 GB available, and every ssh probe answered. Vast read `running` in every
   poll after boot. MEASURED.
4. **Cause, INFERRED.** The hosts' status link to Vast flaps. This was measured on 96607. On 79466 it was not
   seen: every one of the 10 s and 3 s polls over 113 min read `running`. The Worker reads the status far more often
   than that, at every box POST (at least 3 s apart), every runner poll and every cron tick, over about 40 min per run,
   so it can catch a rare flap that our polls missed. That last point is an inference. The Worker takes one non-`running` reading as the end of the rental and destroys a box that is
   probably still alive. Attempt 4 is the strongest case: the box POSTed 4 s before the Worker ended the run (the
   runner's `rent to last box POST 2218s, rent to end 2222s`). A Vast status that turned non-`running` within those
   4 s because of a box death is unlikely (INFERRED, not measured). Attempt 6's 84 s gap without a POST leaves both
   readings open.

## Rentals, 2026-11-06 (all ours, label `kf-vanish-20261106`, each destroyed by exact id with verified absence)

| instance | machine / host | GPU | $/h | created -> destroyed (UTC) | outcome | evidence |
|---|---|---|---|---|---|---|
| 59440610 | 17049 / 96607 | RTX 4090 (1/8) | 0.473 | 16:52:46 -> 16:56:14 | create returned `success: false` with a contract; `vastai start`: "Required resources are currently unavailable, state change queued"; never started | `i59440610_h96607_m17049_never_started/` |
| 59440972 | 17049 / 96607 | RTX 4090 (1/8) | 0.473 | 16:56:27 -> 17:27:00 | never booted in 30 min: `created` with `offline` flaps; daemon log ends at "Waiting for SSH pubkey from server" after two registry `EOF`s; a `vastai reboot` did not help | `i59440972_h96607_m17049_no_boot/` |
| 59446041 | 56506 / 79466 | RTX 3070 (1/8) | 0.134 | 17:27:12 -> 19:20:45 | payload PASS (46 min), then a `merge_check.sh` re-run (24 min, PASS) with 3 s status polling | `i59446041_h79466_m56506_pass/` |
| 59452101 | 55267 / 33035 | RTX 4090 (1/8) | 0.669 | 18:16:03 -> 18:56:10 | payload PASS (37 min) | `i59452101_h33035_m55267_pass/` |

## The two payload runs (MEASURED, tree ecf8c807, 2026-11-06)

| | 79466 / 56506 (attempt 4's machine) | 33035 / 55267 |
|---|---|---|
| provision | rc=0, 823 s | rc=0, 772 s |
| cuda_host | rc=0, 18 s | rc=0, 12 s |
| merge_bar | rc=6, 1457 s, PASS (`V3_GATES_SUMMARY pass=9`, `FAST_SUITE_PASS=30`, `BIRTH_CENSUS_OK`) | rc=6, 1114 s, PASS (same lines) |
| cuda_guest | rc=0, 417 s, cup2/3/8/8bench PASS | rc=0, 288 s, PASS |
| final | `KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4` | same |
| Vast `actual_status` after boot | `running` in every 10 s poll; then 416 of 416 polls at 3 s during the re-run | `running` in every 10 s poll |
| ssh probes (10 s) | 587 of 587 answered (17:29-19:20) | 206 of 206 answered (18:18-18:55) |
| boots in the journal | 1 (no reboot) | 1 (no reboot) |
| min MemAvailable / top RSS | 44113 MB of 50565 / 5.3 GB | 50941 MB of 57793 / 5.3 GB |
| PSI memory some avg10 | 0.00 in every sample | 0.00 in every sample |
| dmesg | Xid 31 only (20 lines, the deliberate fault arms of two merge_bar runs); no OOM, AER, Xid 48/62/79/119/154, hung task or lockup | Xid 31 only (10 lines); same |

The re-run on 79466 (`rerun_merge_bar_keylines.log`): `merge_check.sh ecf8c807`, 18:56:21 -> `EXIT rc=0`
19:20:04, `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, `FAST_SUITE_PASS=30`, `BIRTH_CENSUS_OK`.

Files per run: `payload_keylines.log` (stage and verdict lines), `payload_full.log.gz` (the whole payload output,
scrubbed), `dmesg.log.gz` (streamed live over ssh with `dmesg -w`), `sampler_5s.tsv.gz` (memory, PSI, load, top RSS
and GPU every 5 s), `vast_status*.tsv*` (the Worker's inspect endpoint, status fields only), `ssh_probe.tsv` (uptime
and load every 10 s from the coordinator), `journal_k_warnings_tail.log`. Scrubbed of IPs, UUIDs, boot ids, nonces
and keys. The capture tools (poller, sampler, stream wrappers) lived on the coordinator and are not in the repo;
each is a few lines: a 10 s (later 3 s) `GET /api/v0/instances/<id>/`, a 5 s `free`/PSI/`ps`/`nvidia-smi` loop on
the box, and ssh streams of `dmesg -w` and `tail -F` of the payload log.

## Hypotheses (falsifiers in PREREG.md)

- **H-BOX-OOM/PANIC: falsified for these two runs.** There was no OOM, panic or reboot, and memory headroom was at
  least 44 GB. This does not prove that the Worker's boxes did not die, because no out-of-band capture exists for
  them. The loss signature does not need a box death (row 1 above).
- **H-GPU-FAULT: falsified for these two runs.** The same fault arms raised the same Xid 31 set (box
  `dmesg.log.gz`), with no AER, Xid 79 or "fallen off the bus" line.
- **H-VAST-HOST: supported for host 96607.** On 96607, `offline` status flaps and a VM never got its ssh pubkey
  from Vast's server, while the same image booted in about 2 min on 79466 and 33035. On 96607 the first offer
  could also not be started. Not seen on 79466: in 113 min (payload, idle, re-run; 10 s then 3 s polls) Vast read `running` every time after boot.
- **H-WORKER: supported as the mechanism, from the code.** A single non-`running` reading ends the rental, in
  BOOTING (for anything other than null/created/loading/starting, so `offline` and `exited` too: run 42228490212
  attempt 1 `provider=exited phase=BOOTING`, ended 145 s after rent) and in TESTING (any state). Not yet
  observed live: our direct rentals have no Worker. A direct test is listed below.
- **H-NEW-CODE: falsified.** `ecf8c807` PASSED on attempt 4's machine. `git diff --stat 49da9f9e ecf8c807 --
  crates scripts/bench scripts/fastguest qemu` touches only `scripts/bench/box/windows_gate.sh`, which did not run
  here or in the lost attempts. The first 79466 loss (run 42039008874) was on an older tree.

## Recommendations

Worker (ci/vast-broker, `update()` and `report()`):

1. **Never close on one reading.** In TESTING, end a rental as `ENDED` only when Vast reads non-`running` in at least
   3 consecutive inspections spanning at least 2 min **and** the box has not POSTed since the first of them. A box
   that keeps POSTing is alive whatever the provider says. In `report()`, accept the POST and count the anomaly
   instead of closing. In BOOTING, treat `offline` like `created` until the boot deadline.
2. **Say why.** Print the provider's raw `actual_status` (sanitised to `[a-z_]{1,16}`) and a bounded, sanitised
   `status_msg`, not `unknown`, and add `offline` to the known list. Print one `[Worker] close reason=<R>` line on
   every close (ENDED, UNAVAILABLE, CONTRACT_*, RUN_GONE, TTL, LIMIT, STOPPED, SUPERSEDED). Today RUN_GONE,
   CONTRACT_PRICE/BUDGET, the report-path liveness close, TTL, LIMIT, STOPPED and SUPERSEDED leave no line, and the
   `update()` ENDED close leaves only the `provider=` line (PREREG table).
3. **Keep evidence before destroying.** On a real ENDED, append the serial-console tail (`bootLog`, already
   bounded) to the run log, then destroy.
4. **Host policy.** Count `offline` flaps per host. A host whose instance reads `offline` in BOOTING, or stays on
   "Waiting for SSH pubkey" for more than 10 min, is a provider fault: re-rent on another host. Do not bad-list a
   host for an ENDED in TESTING until rule 1 is in place, or Worker artefacts will exclude good hosts. 96607 earned
   its exclusion from our measurement (instance 59440972, 2026-11-06: no boot, flaps).

Payload: no change is needed for the losses. Optional: print the boot id and uptime in the first report, so that
a box reboot is visible in the run log.

Next test (needs the owner, because it rents through a Worker): run a Worker with rule 1 and rule 2 on a box
whose payload is a 60 min heartbeat loop. `ci/vast-broker/smoke-live.py` already overlays a tiny workload. The
run should go to a flapping host (96607 when offered) and to 79466. *Falsifier:* `provider=offline` lines
appear while POSTs continue and the run survives, which confirms that a live box was destroyed before; a box that
stops POSTing first and then reads non-`running` points back at the host or the box.
