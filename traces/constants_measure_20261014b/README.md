# Constants measurement lane C, resumed (2026-10-14 daytime, branch `claude/constants-measure-20261014b`)

**STATUS: LIVE, 2026-10-14 (second pass; results in `docs/design/V3_CONSTANT_MATRIX_20261014.md` 5.3).** Labels: **[M]** measured by a command whose output is in this
directory, **[I]** inferred, **[U]** not measured. Every hypothesis has its falsifier written before the run.
No UUID, serial, VBIOS string, MAC or PCI subsystem id is in any file here (grep before each commit). Capture files stay on the host
(`/root/kfwc-captures-20261014/`, not in git; `/data` is full), only census text, scrubbed body tables and sidecars are committed.

## Resumption notes

The first lane-C agent died of the weekly limit at ~03:15 CEST. Its Windows rentals (GA104 55835609, GB203 55838542) were destroyed by the
auto-destroy timers with **no capture brought back** (no `/data/kfwc-*` directory exists; the AD103 capture files of
`traces/real_windows_20261014/` are likewise gone from the host, only their census text survives). This directory is the second attempt.

## Falsifiers stated BEFORE the runs (kept verbatim from the first version of this file)

### Windows run on GA104 (RTX A4000, vast KVM VM, Windows 11 LTSC 26100)

Box: vast contract 55921418 (own id), RTX A4000, PCI device 0x24B0 = GA104, host (Linux side, before the reinstall) driver 575.51.03.
**Finding W0 [M]**: `windows-on-vast`'s driver step failed (`setup.exe exited -436207360`) because Windows Update had already installed
NVIDIA **596.51** (`32.0.15.9651`, WHQL, dated 2026-05-07) on its own. So this box runs a driver *newer* than the 580.88 of the AD103/AD104 captures,
i.e. a different GSP branch. That is a bonus for F-4 (595-branch request shapes) and a deviation from the like-for-like 580.88 reference.

| # | hypothesis | falsifier (written before the run) |
|---|---|---|
| W1 | the 596.51 GeForce driver runs the A4000 on GSP when `EnableGpuFirmware=1` is in both keys and the adapter is restarted | `nvidia-smi -q` GSP Firmware Version still `N/A` after the restart, or the collector attaches to nothing |
| W2 | the mailbox route attaches from `seqNum` 0 on this die and driver | `prefix_missing`, first `seqNum` != 0, `seq_gap` or `parse_failure` on the mailbox-attached instance (`kf-census`) |
| W3 | F-4: the controls 852e/853a/a618/852f/853b/b201 changed between firmware branches (host 595.91.07 answers them INVALID_ARGUMENT to the 580-branch requests) | the 596.51 driver sends the same all-zero requests as 580.88 does: then the branch explanation is wrong and the 595.91.07 refusal is a host-driver property |
| W4 | GA104 and AD104 GMMU static info differ (`0x20800a59`); kayfabe's authored GA106-equal fault-buffer sizes are not the GA104's | the GA104 reply equals the authored/AD104 values |

### Windows run on GA106 (RTX 3060, vast KVM VM, Windows 11 LTSC 26100, NVIDIA 580.88)

Box: vast contract 55929605 (own id), PCI device 0x2504 = GA106, driver 580.88 (`32.0.15.8088`, installed by `windows-on-vast`; the same
package as the AD103 reference), 51.5 GiB RAM. GA106 is the die kayfabe's authored/captured constants were taken from.

| # | hypothesis | falsifier (written before the run) |
|---|---|---|
| X1 | with `EnableGpuFirmware=1` the 580.88 driver runs GA106 on GSP 580.65.05 and the mailbox route attaches from `seqNum` 0 | `nvidia-smi -q` GSP version `N/A`, or `prefix_missing` / `seq_gap` on the mailbox-attached instance |
| X2 | GA106 request sizes equal the AD103 580.88 sizes (852e 528, 853a 1456, a618 8620, 853b 1832, b201 2188) | any of them differs (then the size is die-dependent, not branch-dependent, and W3's reading is wrong) |
| X3 | `INTERNAL_GMMU_GET_STATIC_INFO` = 0x31000 / 0x120c20 and `CE_GET_FAULT_METHOD_BUFFER_SIZE` = 0x5000 (kayfabe's authored numbers were "chosen equal to a stock GA106 GSP") | either differs |

### Tier-2 and Windows-branch hypotheses (falsifier written before the probe was added to the kit)

| # | hypothesis | falsifier | result |
|---|---|---|---|
| H1 | F-4: a 595.91.07 host accepts the zero request at the 596-branch sizes (852e 596, 853a 15352, a618 8632, b201 2444) | `INVALID_ARGUMENT` at any of them | **not falsified** [M]: all OK on AD104 and AD103 at 595.91.07, L4 at 595.84, GB203 at 595.91.07 |
| H2 | the sizes are a property of the firmware branch, not the die | the same host branch accepts different sizes on two dies | **not falsified** [M]: 15 (host driver, board) cells, every branch consistent across dies |
| H3 | the 580-host sizes (528/1456/8620/2188) are accepted by every 580.x host | a 580.x host refusing one | not falsified [M] (7 hosts) |
| H4 | Windows driver N.xx runs the GSP of the highest R-branch <= N | a GSP string outside that branch | **not falsified** on 8 builds [M] |
| H5 | the ZBC default entries are the same on every die | two dies with different leading entries | **falsified** [M] for Turing (formats 1/2/4, colorDS) vs Ampere/Ada/Blackwell |
| H6 | `0x20808162` is a per-die value | the same die answering differently on two boards | **falsified** [M] (GA102 3090 = 01, A5000 = 00): it is board-class, F-6 |
| H7 | GMMU static / CE fault-method buffer are the GA106 numbers on every die | a die answering otherwise | **falsified** for AD104 (5.1); not falsified for GA104 (595.76) and GA106 (570..610) |
| H8 | a second restart in one collector session keeps the module attached | collector exits early | falsified once (the SSH session closing stopped the service; rerun with one restart) |

## Files

* `tier2/raw_<offer>.txt` — output of `scripts/bench/constants/tier2_probe_box.sh` on one container rental (unprivileged, `CapEff` without CAP_SYS_ADMIN). `GA104_*_{unpriv,admin}.txt`: the same on a KVM box before Windows replaced its Linux, with and without CAP_SYS_ADMIN. `../tier2_summary.tsv`, `../shapes_by_host_driver.tsv`: `summarize_tier2.py`, `shapes_by_host.py`.
* `ga104_596.51/`, `ga106_<windows build>/`: census text of the kfwc captures (`*.controls.tsv` counts/sizes/statuses, `*.functions.tsv`, `*_scr.bodies.tsv` = bodies of the allow-listed controls only, `seqs.txt`/`binds.txt`, `cell.json` sidecar, capture sha256). **The capture files themselves stay in `/dev/shm/kfwc-captures` on the host and are not in git.**
* `ga106_sizes_by_windows_driver.tsv`: request-size set of all 216 controls across seven GSP branches; `branch_sizes_580_vs_595.tsv`: two-run version.
* `../../scripts/bench/constants/`: `tier2_probe_box.sh` (kit), `run_container_probe.sh`, `process_capture.sh` (capture -> repo-safe text), `windows_rows.py`, `compare_runs.py`, `sizes_matrix.py`, `shapes_by_host.py`, `summarize_tier2.py`, `gen_rows.py` (table rows from bodies).

## Windows runs (what ran, in order) (falsifiers: section above)

| box (own id) | die | what | result |
|---|---|---|---|
| 55921418 | GA104 RTX A4000 | WU installed 596.51; GSP on; captures C, E | W1 holds (GSP 595.76); W2 holds on the restart-created instance (seq 0..5179 / 0..5216, 0 gaps); instance 1 (scan-attached) has `prefix_missing` as announced; W3 holds in the strong form (sizes change); W4: GMMU = authored (not AD104's). Then the downgrade attempt to 580.88 with the GSP loaded left the GPU in Code 43 across a VM restart; box destroyed. |
| 55929605 | GA106 RTX 3060 | 580.88 installed by windows-on-vast; captures C, E | X1 holds (GSP 580.65.05, seq 0, 0 gaps); X2 holds (528/1456/8620/2188 = AD103's); X3 holds (0x31000/0x120c20, 0x5000) |
| 55944979 | GA106 RTX 3060 | six more Windows builds on one box (see matrix 5.3.1) | all PASS from seq 0; Windows -> R-branch mapping verified on all |
| 55937420 (CN) , 55936078 | GB202 RTX 5090 | Windows install | CN box never started (image pull 20 min); Ontario box sat at the Windows Boot Manager for 45+ min, no SSH: destroyed |
| 55941689, 55959274/292 | GA102 RTX 3090 | Windows install | `GPU error, unable to start instance` (also the AU 3090 earlier); a duplicate create (`success:false` with a `new_contract`) is a vast API quirk, destroyed |

## Mistakes and traps (so they are not repeated)

* `vastai create instance` can print `"success": false` and STILL have created the instance (seen 5 times). Parse `new_contract`, then check `vastai show instances` for the label. One of those unfiltered outputs printed the per-instance `instance_api_key` into my terminal once (not saved anywhere, instance destroyed).
* `pkill -f '<pattern>'` kills the calling shell if the pattern is in its own command line.
* The kit's `ls /dev/nvidia[0-9]* | head -1` picked a node the container's device cgroup denies (EPERM) on 8 boxes; fixed to the first node `: < node` can open.
* `windows-on-vast`'s `setup.exe -s -n Display.Driver` exits -436207360 when a driver is already present; the DriverStore route (5.3.1) works.
* `/data` and `/` of the host were at 100 %: captures live in `/dev/shm` (RAM), cargo targets too.
