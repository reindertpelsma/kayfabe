STATUS: LIVE, 2026-11-18. MEASURED on three rented Vast boxes (nested KVM guests; evidence is text only: logs, JSON, TSV, TXT). This directory is the evidence of the acceptance run of 2026-11-17 and 2026-11-18, a strict (`propimages_strict: true`), zero-skip, release-gate-style attempt, with all big data fetched only through presigned GET URLs minted for the box (read-only credentials never left the coordinator; the URL file is not part of this directory). **It is NOT a PASS claim: the run was stopped at the owner's pace limit before the new final tree was run. The verdict, per-stage table, named deviations and the resume list are in the result file of the run (`ACCEPTANCE_RUN_RESULT_20261117.md`, kept by the coordinator; its sections are summarised below).**

## Boxes (die, driver, memory)

| dir | box | die | host driver | CPU / RAM (MemTotal, MemAvailable) | tree (git sha) |
|---|---|---|---|---|---|
| `box1/L1..L5` | first box, offline three times, destroyed by the coordinator | RTX 4090 (AD102) | 580.159.04 | EPYC engineering sample, 30 vCPU, 51762156 kB total, about 47.6 GiB available | L1 d33d20c0b, L2 and L3 021cad3cc, L4 021cad3cc (image B), L5 049fbf3cb |
| `spec/S1`, `spec/S2` | SPEC lanes | RTX 5070 Ti (GB203, Blackwell) | S1 580.159.04 (refused by kf3), S2 595.91.07 | Ryzen 9 5950X, about 98 GB | 049fbf3cb |
| `w1/W1a`, `W1m`, `W1c`, `W1b2` | wave 1 | RTX 4090 (AD102) | 580.159.04 | Ryzen 9 5900X, 51547780 kB total, 49968800 kB available at the end | W1a and W1m 2fab69396 (provision, first matlab attempts), W1c 46277bb7b (image A), manual matlab and W1b2 (image B) 8c04fbd34 |

A kfml large-tier re-run on a 125 GB container box (RTX 4090 48 GB variant, driver 595.71.05; managed_oversub 6 of 6 PASS) is summarised in the result file; its box was destroyed and its evidence is not in this directory. NONE of these trees contains PR #480 (the host_pte_kind depth-kind fix); the new final tree (`claude/accept-final-tree2-20261118` at 0bacf4a0a: #480, #490, #508 merged first) was not run.

## Legs

| leg | content | result (details in the result file) |
|---|---|---|
| `box1/L1` | first 14-stage invocation | red at `apps` (llama.cpp Web UI download at build time; fixed by PR #469) |
| `box1/L2` | restart on a dirty box | red at `merge_bar` (stale host Xids after a restart; box reboot before a relaunch) |
| `box1/L3` | restart | provision, cuda_host, merge_bar, cuda_guest, apps 66/66, kfml (managed_oversub NOT_RUN: 47.6 GiB), install_flow, windows_gate, windows_apps 10/10, windows_gate_2 PASS; image A refused (swtpm build missing in the generic runner, PR #476) |
| `box1/L4`, `box1/L5` | image B gate rows, matlab first attempt | image B rows HARNESS "Task timeout" (not reproduced later), matlab not reached |
| `spec/S1` | SPEC on the RTX 5070 Ti, host driver 580 | refused by kf3 (no measured display PRECOMP rows for GB203 on branch 580; PR #485) |
| `spec/S2` | SPEC both lanes, host driver 595.91.07 | H1 holds (licence `Licensed` at boot ages up to 93 min); energy-04 PASS, creo-04 and enscape-01 host Xid 32, blender-01 guest lost; 5 of 9 viewsets NOT RUN (lane cut) |
| `w1/W1a`, `w1/W1m` | wave 1 provision, image A attempts, matlab attempts | GOP missing in the generic runner (PR #491), matlab not on PATH (PR #500) |
| `w1/W1c` | image A, strict, tree 46277bb7b | PASS: gate 2/2 (solidworks, edrawings), discovery superposition and 3dmark PASS, resolve INCONCLUSIVE (Resolve.exe crashed in nvoglv64.dll; host Xid count 0 on that box) |
| `w1/W1b2` | image B gate lane with the guest diagnostics (`diag3_b_*.txt`; `diag_b_70.txt` is the first attempt whose stdin script printed nothing) | 6 of 7 rows pass (lightroomlaunch FAIL: app did not exit cleanly); the earlier timeouts did not reproduce |
| `w1/W1b2/workspace/matlab_manual3` | MATLAB by hand, shared isolation, tree 8c04fbd34 | stage PASS, 2 rows PASS, 5 rows GPU_NOT_USED (graded PASS by census, see the PRs #509 and #514), parpool_gpu LicenseUnavailable |

## What is in a leg directory

`workspace/` mirrors the box: `payload_stdout_<leg>.log`, `payload_exit_<leg>.line`, `payload_start_<leg>.marker` (the exact payload arguments), `kf-ci-evidence/` (stage JSON and run lines), stage prefixes (`kf-propimage*/`, `kf-matlab/`, `kf-kfml/`: row verdicts, census, boot logs, host `nvidia-smi` samples), `bench/` (xid-watch logs, provisioning logs), `root/prov/` (windows gate and SPEC stage evidence). Bulk files are not here: no image, no screenshot, no archive, no QEMU command line, no kernel log of the whole box, no URL file.

## Scrubbing (applied by script, then re-checked by search)

GPU UUIDs are replaced by an all-zero UUID; presigned-URL query parameters by `X-Amz-<scrubbed>`; e-mail-shaped strings by `<email>`; licence-key-shaped strings by `<key-shaped>`; IPv4 addresses other than loopback and private ranges by `<ip>`. No account, identity or licence value of an image is in this directory, and pins (public hashes) are the ones already in the repository. A reader who finds one of these classes of value in a file should treat it as a scrubbing defect and say so.
