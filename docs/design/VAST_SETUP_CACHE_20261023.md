# Vast setup dependency cache — V8

MEASURED on 2026-10-23, run `v7-kvm-20261023` (source `28084b96cebad2a853d72171f72f8d776946b8ba`): the saved timings below come from [V7 evidence](../../traces/vast_payload_v7_20261018/LIVE_RUN.md); the destroyed box was not queried again. V8 rental values remain pending.

| Phase | V7 seconds | V8 seconds | Evidence / limit |
|---|---:|---:|---|
| Provider create → running | 904.048 | pending | Provider polling interval: 20 s |
| Create → first report | 1954.956 | pending | V7 required an unchanged-source manual restart |
| provision_box | 3295 | pending | 02:00:03–02:54:58 UTC |
| Before APT completion | 3232 | pending | Includes preflight/update/download; not an exact APT command duration |
| dpkg configuration | 57 | pending | APT history 02:52:58–02:53:55; excludes downloads |
| Toolchain/targets + workspace build | ≤63 | pending | Remaining provision_box window; individual times not recorded |
| Host driver | 237 | pending | Open 580.159.04 |
| Bench host dependencies | 904 | pending | Existing provision_bench_tree host APT |
| QEMU track A | ~87 | pending | Unpack/Rust/Ninja included; A_RC=0 |
| Nested fat guest preparation | 207 | pending | Ubuntu 24.04 guest; separate from the host cache |
| Musl build | 23.30 | pending | V7 then failed at an adapter output-path mismatch, since fixed |
| Dependency-cache restore | absent | pending | V8 reports download/extract/install and total seconds |
| Payload entry → READY | not reached | pending | Target approximately 900 s; measured independently of provider boot |

MEASURED on 2026-10-23, run `v7-kvm-20261023`: the saved archive snapshot contained 327 complete .deb files / 1,355,662,990 bytes, including the image baseline; the transaction-only count/size was not preserved. Release-file probes returned 346,021 / 1,849,679 / 1,336,943 B/s for archive / Azure / edge; Azure was chosen. These small-file probes do not establish bulk throughput.

CODE READ: `provision_box.sh --packages ci` lists the required build/driver/initrd tools; the default manual `full` profile additionally keeps clang, lld and libssl-dev. The reviewed Cargo locks have no OpenSSL/native-tls dependency, and the required flows use GCC; the firmware linker comes with Rust. `--packages bench` shares the existing QEMU/guest-image dependency list with the producer. The payload forces `ci` and excludes APT recommendations. No tested project binary is cached: every checkout rebuilds its exact SHA.

MEASURED on 2026-10-23, run `v8-cache-build-20261023`, [producer evidence](../../traces/vast_setup_v8_20261018/cache_build.json): an authenticated Ubuntu 22.04 empty-dpkg-state APT plan resolved 255 packages / 158,286,760 bytes. Six concurrent HTTPS downloads took 6.786 s; the producer took 272.302 s including per-package signed-index metadata lookups. Concurrent 8 MiB / maximum 10 s bulk probes chose edge (0.124457 s), versus archive (1.044052 s) and Azure (timeout, no bytes). This was the producer machine, not the acceptance host.

MEASURED on 2026-10-23, run `v8-cache-offline-20261023`: a fresh Ubuntu 22.04 container with networking disabled installed that closure in 14.330 s after a 0.040 s local-index update. This is a local installation check, not the rental setup measurement. [Published cache evidence](../../traces/vast_setup_v8_20261018/cache_publication.json) records both downloaded-and-verified release assets: 158,145,005-byte .deb archive and 7,888,026-byte Cargo vendor archive (54 crate directories; root and firmware locks).

CODE READ: the one-off owner-invoked producer is `bash scripts/ci/build_setup_cache.sh /absolute/new/output setup-cache-<tag>`; it uses the digest-pinned Ubuntu 22.04 producer Dockerfile, signed APT SHA256 metadata, bounded six-way prefetch and `cargo vendor --locked --sync firmware/kf-gop/Cargo.toml`. Review the generated lock, publish the two assets to the private store with `gh release create`, and commit the reviewed lock. This flow has no PR trigger, rental or private-store credential inside the producer container.

CODE READ: the production tiny onstart remains unchanged. Repo-side onstart installs `kf-box-blob-url`; the dependency consumer requests two run-scoped locators, keeps each signed URL out of logs/argv, permits only fixed GitHub CDN HTTPS hosts without redirects or credentials, and checks exact size plus SHA256 before extraction. Extraction rejects traversal, links, special files and expansion beyond 8 GB. Existing extraction directories are preserved on refusal. No store PAT, Vast key, App key or owner stop token reaches the box.

CODE READ: `setup-cache.lock.json` binds intended KVM image, Ubuntu 22.04/x86_64, package policy and both Cargo locks. OS/architecture checks are compatibility checks, not host attestation. Each asset is below 2 GB. The box is untrusted; approved code plus reviewed hashes, not a claim of provider trust, establish the chosen cache contents. The project remains an untrusted GPU tripwire.

CODE READ: at boot three bounded bulk probes record network conditions; restored host packages actually come from the local file repository. APT disables all global source fragments and recommendations, uses an isolated list directory, host queues and pipeline depth 10. Cargo replaces crates.io with the hash-checked vendor directory and runs offline. Ubuntu 24.04 guest APT and driver/toolchain/image downloads keep their existing flows and are timed separately. Missing, stale or corrupt cache data produces structured FAIL/setup_cache rather than falling back to unbounded network installs.

INFERRED: avoiding the two host dependency downloads should remove the dominant V7 delay. Falsifier for the fresh lease: dependency restore plus provision_full to READY still exceeds about 900 s, or any required build dependency is absent. The single V8 lease must also show unattended first reporting and MANUAL retention beyond the saved GRACE deadline; no source restart is permitted for those claims.

MEASURED on 2026-10-23, run `v8-gpu-free-20261023`, [preflight evidence](../../traces/vast_setup_v8_20261018/preflight.json): 282 GPU-free tests pass. The standalone EXPECTED RED commits precede the implementation (package policy, forced CI profile, cache identity/extraction, APT metadata/probes, blob-helper port, entry order, recommendations, epoch filenames and APT hook). Explicit fixture migration adds the new helper to both canonical copy lists; production inline-onstart expectations are not restored. The new cache/policy and existing payload/environment/diagnostic mutation ledgers name at least two failing tests per mutant. Canonical 60-case mutation results are recorded separately under their original rule.
