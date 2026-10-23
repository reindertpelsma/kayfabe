# Cloud-init replay and one further KVM acceptance

MEASURED on 2026-10-23, run `v8-cloud-init-gpu-free-20261023`: [325 tests](preflight.json),
96 noncanonical mutations each with at least two named killers, canonical [60/60](canonical.txt).
Components: payload29 + provider-env8 + diagnostic10 + package-policy6 + cache/helper/home43.
The new [policy mutations](policy_mutants.json) cover the SSH package and skip-APT probe;
[cache mutations](cache_mutants.json) retain the existing hash/origin/deadline/ACK guards.
Standalone EXPECTED RED commits: 4238e09fd (missing package), c1b74a3d8 (probe skips install).

MEASURED on 2026-10-23, run `v8-cloud-init-replay-20261023`: two fresh, network-disabled
Ubuntu22 containers pass their scenario assertions ([results](replay.json)): a complete fixture
PASS (73.729s), and an injected post-driver-swap provision FAIL (55.325s), reporting driver
580.159.04 after initial575.51.03 and never starting a later CUDA stage. Root; non-login,
non-interactive shell; no HOME/USER/LOGNAME/TTY; no systemd; KF data only in /etc/environment.
Actual entry, cache validation/extraction, offline APT, provision_box/provision_full, host-driver,
bench-tree, CUDA and merge orchestration execute. GPU/kernel/VM/compiler/deep-suite operations
are explicit recorded fixtures. These are orchestration tests, not GPU evidence.

CODE READ: `scripts/ci/cloud_init_replay.py` refuses execution outside its isolated checkout,
manual container guard or network-disabled namespace. Run a new disposable container from
`scripts/ci/setup-test.Dockerfile`, with `--network none --cpus 2 --memory 2g`, this source copied
to /workspace/kayfabe, the two lock-matching blobs mounted read-only under /fixtures, an empty
/output mount, and KF_CLOUD_INIT_REPLAY=1. Invoke `/usr/bin/python3
/workspace/kayfabe/scripts/ci/cloud_init_replay.py pass` or `swap-failure` in separate containers.
Only the loopback TLS fixture broker runs; no account credential or real run token is needed.

MEASURED on 2026-10-23, run `v8-cloudinit-cache-publish-20261023`: [v2 publication](cache_publication.json)
adds openssh-client and its dependencies. Both assets were re-downloaded, including with the
[read-only store credential](cache_ro.json); bytes/SHA256 match the reviewed repository lock.

CODE READ: the owner authorised one further live lease at 10:45, cap4/stop3 USD, after this
replay. No rental has started at this checkpoint. The live source SHA and outcome will be
recorded here. [Prior fresh run](../vast_setup_v8_fresh_20261018/LIVE_RUN.md) remains FAIL/provision.
Worker bundle remains 2402798ce8ab53dcc4db5efca7891b565c0bca7e92723ff0b5050e5bb6e6760f.
[Pre-recorded hypothesis/falsifier](experiment_plan.json). No deployment/production enablement.
