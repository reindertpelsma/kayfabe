# propimage_linux slice 1: local build, seal and offline-clone proof (no application, no GPU, no Vast)

STATUS: LIVE as evidence, 2026-11-16 (text only; the 2.3 GiB image and the run directory stay on the coordinator box and are deleted).

Run: `/workspace/imagec_20261116/proof` on the coordinator box (Scaleway VM, 6 vCPU AMD EPYC, nested KVM, QEMU 10.2.1), 2026-11-16 between
about 19:10Z and 19:50Z (guest clock; host wall clock 20:xx local). Tooling revision: the branch `claude/propimage-linux-tooling-20261116` before its
final commit (parent `26805e3f9`, the design). Seed `kayfabe-linux-proof-20261116` (a throwaway test seed), 4096 MB, 2 vCPU, 30 GB virtual disk, desktop image,
base `ubuntu-24.04-server-cloudimg-amd64.img` `release-20261017` sha256 `04e8f246...b6c94`.

| File | What it is |
|---|---|
| `setup.log`, `setup_argv.txt` | `setup`: unattended provisioning, timings, the identity probe |
| `seal.log`, `seal_report.txt` | `seal` with a test pattern set: counts only, per pattern label and per guest path |
| `scan_image.log` | `scan-image`, the independent reader, on the sealed qcow2 |
| `run_clone1.log`, `run_clone2.log`, `run_argv.txt` | two separate offline clones: identity, no network, recorded clock |
| `offline_check_reboot.log`, `offline_check_plus35.log` | `offline-check` mechanics (E1 with one reboot, E3 with +35 days) with `echo` commands, not MATLAB |
| `auth_selftest.log` | the sign-in capture helper against the stub app on an offline clone (loopback x3, paste) |
| `vnc_ws.log` | noVNC's websockify reached the clone's VNC (HTTP 101 upgrade) |
| `pack.log`, `unpack.log` | zip round trip (2 463 371 583 bytes), sha256 per member checked |
| `packages.tsv`, `state_summary.json` | package manifest (1645) and the state without secrets |

No application, no licence, no credential was involved. The grep patterns were a test set; the report holds counts and paths only.
