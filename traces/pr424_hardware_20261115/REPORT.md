# PR #424 hardware validation, one Vast box, 2026-11-15
STATUS: LIVE, 2026-11-15. MEASURED on one box (RTX 4090 / AD102, AMD EPYC 7V73X), local evidence only: no Worker, no run token, no attested setup cache (OWNER_RULINGS AR item 4). Not a release record.
Evidence: branch `claude/pr424-hardware-20261115`, draft PR against next, `traces/pr424_hardware_20261115/` (README there). Local copy: `/workspace/bench/pr424_hardware_20261115/`.
Nothing in PR #424 was edited.

## Bottom line
- Every stage that was run is GREEN on the PR head, and the kernel self test passes on the GPU (82/82, the rewritten `hostile/report_run_cap` included).
- **The run did not exercise the new code paths in the real stack.** No `walk report CUT`, no `SLOW host unmap call`, no `SPLIT-LATE`, no `TRUNCATED`: the largest apply was 12,288 maps (319 ms), the
  largest unmap 2,008 runs in 2 ranges (34 ms), far below the 4,096-entry call cap and the 262,144-run report capacity. So this is a no-regression result for the gates, plus an on-GPU result for the kernel
  on small shapes. It says nothing about whether the cap or the cut changes the Wukong TDR rate (needs a diff above 262,144 runs; see "Unproven").

## Run
- sha `9ddb897e7281ecff4baa9d35acb863bbd265644a` (PR #424 head; base next 84dff90b6 + 4 commits). Fetched on the box from GitHub by that sha; `git rev-parse HEAD` matched. Worktree locally: `/workspace/wt-pr424hw-20261115`.
- Instruction `instruction.json`, sha256 `63ab6fbeb11f83d43c1e8af991ae774dcdd3c5680727a7cf4f8f4e1bc2cc6c34` (also in `kf-result.json`). `--dry-run` plan on the coordinator: `provision:merge_bar:cuda_guest:windows_gate`
  (`prep_dryrun.txt`). Run with `--only provision --only cuda_host --only merge_bar --only cuda_guest --only windows_gate`: DEVIATION, `cuda_host` (13 s) added so `host_reference` is GREEN.
- Same deviations as the earlier rehearsal: `python3 -I vast_payload.py` directly (no `vast_payload.sh`), apt and cargo online, no secrets on the box. First real use of `--only` on a box: it worked (`selection` is in the result file).
- Box: Vast 60811732, `kf-pr424-20261115`, offer 60193276 (Romania), 30 vCPU, 98 GB, disk 194 GB (186 free), `/dev/kvm` present, KVM image, driver 575.51.03 -> 580.159.04 by provisioning. Label rule met.
- Two rentals before it were destroyed unused (60809802, then 60811069, offers on a Denmark host): the forwarded ssh port never answered (connect timeout / refused, ping 100 % loss from here for the IP, KVM guest
  itself booted per its console log). Not a kayfabe matter; INFERRED host network problem. Together about 0.27 USD.
- Window: payload 16:53:32Z to 18:48:10Z (1 h 54.6 min), `EXIT rc=0`; box 16:50:46Z to destroy 18:53:59Z (2.06 h).

## Per-stage verdicts (`kf-result.json`: verdict PASS, host_reference GREEN, all rows fail=0 skip=0)
| Stage | Duration | Verdict and evidence |
|---|---|---|
| provision | 11.1 min | PASS, `READY driver=580.159.04` (`prov/prov.log`) |
| cuda_host | 13 s | PASS 4/4 |
| merge_bar | 21.2 min | PASS: `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, `KF3_RC=0`, `BARE_SUITE_PASS=30`, `FAST_SUITE_PASS=30`, `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12 privilege=USER` (`prov/ci42865825151a12e7.log`). Stage rc=6 is the Xid watcher's design exit, graded PASS (as in the 84dff90b rehearsal) |
| cuda_guest | 5.0 min | PASS 4/4 (cuda_fat) |
| windows_gate | 77.0 min | `WINDOWS_GATE_PASS rev=9ddb897e ... idle_s=300 resets=0 known_live_dumps_193=1 live_dumps_193_entries=3 ratchet=PASS die=AD102 stale_lke_resubmits=0`; install 2970 s (`prov/..._windows.log`, `winvm/`) |
Windows LKE: one `Kernel_193` LiveKernelEvent (WATCHDOG dump, guest 18:34:36Z) with `entries=3 resolved=1 unresolved=0`, the same counts as the 84dff90b rehearsal (`known_live_dumps_193=1 (entries=3)`). The gate ratchets it as
known. INFERRED, not shown: no host-side slow apply or `SPLIT-LATE` exists in either gate QEMU log, so nothing links it to the VA manager; the Windows idle here has no Wukong-sized mapping load.

## Xid (kernel.log 138,410 bytes, 1,144 lines; offsets in `stage_kernel_offsets.tsv`)
| Stage | kernel.log bytes at end | Xid lines at end |
|---|---|---|
| provision | 65,654 | 0 |
| cuda_host | 65,654 | 0 |
| merge_bar | 136,429 | 10 |
| cuda_guest | 136,871 | 10 |
| windows_gate | 138,410 | 10 |
All 10 are `Xid 31` in merge_bar and are the deliberate-fault arms, same signatures as the earlier rehearsal: raw suite `kayfabe-rm-ladd` pid 64012, ch 0x4/0x4/0x5/0x4, CE0 HUBCLIENT_CE1 FAULT_PTE at
0x60_00010000, 0x62_.., 0x62_.., 0x64_.. (17:07:39 to :48Z, arm defer-liveness) and pid 64437 ch 0x5 GR0_PBDMA0 FAULT_PDE WRITE at 0xa_11000000 (17:08:26Z, missing-page-fault); fast suite `qemu-system-x86`
pid 67883 (17:14:23 to :32Z, same four addresses) and pid 69592 ch 0x1b FAULT_PDE at 0xa_11000000 (17:17:05Z). The payload's own allow-list admitted them (graded merge_bar PASS). **No Xid outside the deliberate-fault arms**; none in
cuda_guest or windows_gate. No oops, hung task, OOM, soft lockup or reset line in kernel.log (grep count 0).

## kf3 log lines per stage (`collect_final/`, 188 bench logs + prov + Windows gate logs; counts are lower bounds, see README for the log caps)
| Line | merge_bar (raw, fast) | cuda_host / cuda_guest | windows_gate |
|---|---|---|---|
| `walk report CUT` | 0 | 0 | 0 |
| `walk report TRUNCATED` | 0 | 0 | 0 |
| `SLOW host unmap call` | 0 | 0 | 0 |
| `SPLIT-LATE` | 0 | 0 | 0 |
| `walk capacity ceiling` | 0 | 0 | 0 |
| channel `DEAD` (`chan token ... DEAD`) | 0 | 0 | 0 |
| `no run of its window applied`, budget (`windows ... budget 64`) | 0 | 0 | 0 |
| `SLOW-APPLY` | 0 | 0 | 0 |
(The word DEAD also appears in the deliberate `defer-liveness` arm text, `copy=DEAD rescue=LIVE`; that is the arm's own line, not a channel.)
- CUT windows: 0; maximum windows per request: 0 (the number is not logged outside the budget failure). Maximum SLOW host unmap call: none (threshold 250 ms). TRUNCATED or budget failure: none.
- `kf3: mem large apply` (9 lines): max 12,288 maps in 319 ms (`cup3`, cuda_host/guest lane); Windows gate boot: max 2,654 maps in 131 ms, one unmap apply of 2,008 runs in 2 ranges, 34 ms.

## kf_tests on the GPU (`kf_tests/`)
- The box had no toolkit (`nvcc` absent after provision); installed `cuda-nvcc-12-2` + `cuda-cudart-dev-12-2` 12.2.140-1 from NVIDIA's apt repo (same release as the pinned NVRTC 12.2.140). RTX 4090 sm_89, driver 580.159.04, JIT from the PTX nvcc emits for compute_75.
- `make check-invariants` rc 0; `make kf_tests` rc 0 (warnings flags on); `./kf_tests hostile/report_run_cap`: `pass hostile/report_run_cap`, 1 passed 0 failed; full `./kf_tests`: **82 passed, 0 failed** (includes `race/gpu_live_edits`,
  `race/gpu_garbage`, the round-trip diffs); `make check-ptx`: "jit ok: PTX-only binary" (its `cuobjdump -ptx` line printed `cuobjdump: not found`, toolkit package lacks it: cosmetic, no SASS check beyond the grep).
- The shipped artifact, the NVRTC-built committed `cuda/walk/kf_walk.ptx`: `cuModuleLoadData` JIT loaded it on this GPU and all 14 entries resolved (`kf_tests/committed_ptx_jit.txt`); `ptxas -arch=sm_89` accepted it (`ptxas_sm89.txt`).
  NOT run: a launch of the committed PTX's `kf_diff_emit` cut path. `kf_tests` builds the kernel from `kf_walk.cu` with nvcc, it does not load the committed PTX.
- Not run here: `make check-cpu` and the mutant scripts (GPU-free, the PR and the review ran them).

## Cost and teardown
- Box rates 0.854 (60809802, 13.1 min), 0.798 (60811069, 6.5 min), 0.959 (60811732, 2.06 h) USD/h: about 0.19 + 0.09 + 1.97 = **2.2 USD** (offer rate times time, not an invoice; the account credit fell 2.21 USD over the period, other agents may share it).
  Total elapsed 16:30:54Z to 18:53:59Z = 2.4 h. Hard stops were 5 USD / 5 h.
- Teardown on 2026-11-15 (RTX 4090 box, instance 60811732, run of rev 9ddb897e7): `vastai destroy instance 60811732 -y` at 18:53:59Z, then `vastai show instances-v1 --raw`: only `60824157` (not mine) remains; mine absent (evidence `teardown_verify.txt`). The same listing check was made on 2026-11-15 after each of 60809802 and 60811069 was destroyed (`log.txt`). No other instance touched.
- Evidence copied back: text, logs and JSON only (`export.tgz` 0.66 MB); no tarballs of images, no PNG. Scrubbed (GPU UUIDs, guest serial, MAC); `no_gpu_uuids.py` and `no_private_refs.py` pass; `doc_freshness.py` clean.

## What remains unproven
1. Everything the PR's section 2 to 4 asks of a box that needs a big diff: a real `CUT` window, a split that completes after a CUT, the largest `SLOW host unmap call`, `SPLIT-LATE` clusters against the 2 s TdrDelay, the TDR/LKE count per Wukong run.
   No stage here produces a diff above 262,144 runs or an unmap above 4,096 entries. Wukong 4096 MiB Very High, 3 runs, needs the proprietary image: not run.
2. The committed PTX's cut path on a GPU (only a JIT load; `kf_tests` covers the same source via nvcc) and the GPU time of `kf_diff_emit` on a cut report (not measured).
3. Whether chunked unmap helps the TDR rate (total apply time unchanged by design).
4. `walk report CUT` and `SLOW host unmap call` logs are capped (32 / 16 lines per process) and the windows-per-request maximum is not logged; a Wukong run should not rely on these counters alone (the PR notes the cheap follow-up: record the largest clump and `fcut` in the line).
5. One die, one box, one run; the 84dff90b rehearsal is the comparison (same verdicts and counts).
Suggested next box run for item 1: a Linux fat-guest or Windows workload that maps and frees several GiB with 4 KiB pages on kf3, or the Wukong benchmark on the proprietary image, with `KF_VAS_CENSUS=1`.

## Harness notes (not fixed)
- `--only` on a real box works; `--dry-run` printed the selection without `cuda_host`, which is why it was added by hand.
- The toolkit is not installed by provisioning (the task text assumed it): an on-box `kf_tests` run needs an apt install step, or a stage (`cuda/walk` `make check` is not part of any payload stage).
- The same H2 cosmetic as before: `merge_bar rc=6` printed beside `result=PASS`.
