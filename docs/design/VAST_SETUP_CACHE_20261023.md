# Vast setup dependency cache — V8

STATUS: LIVE, 2026-10-23.

MEASURED on 2026-10-23, run `v7-kvm-20261023` (source `28084b96cebad2a853d72171f72f8d776946b8ba`): the saved timings below come from [V7 evidence](../../traces/vast_payload_v7_20261018/LIVE_RUN.md); the destroyed box was not queried again. V8 used source commit ff29c4a83a1bd359352263e135573c8167c298b6; its [rental evidence](../../traces/vast_setup_v8_20261018/LIVE_RUN.md) records a fail-closed readiness race before package work.

| Phase | V7 seconds | V8 seconds | Evidence / limit |
|---|---:|---:|---|
| First independent running poll | 904.048 | 145.918 | V7: from create response; V8: from rent-start; both 20 s polling |
| First accepted report | 1954.956 | 130.610 | V7: from create response, manual restart; V8: from rent-start, unattended |
| provision_box | 3295 | NOT RUN | 02:00:03–02:54:58 UTC |
| Before APT completion | 3232 | NOT RUN | Includes preflight/update/download; not an exact APT command duration |
| dpkg configuration | 57 | NOT RUN | APT history 02:52:58–02:53:55; excludes downloads |
| Toolchain/targets + workspace build | ≤63 | NOT RUN | Remaining provision_box window; individual times not recorded |
| Host driver | 237 | NOT RUN | Open 580.159.04 |
| Bench host dependencies | 904 | NOT RUN | Existing provision_bench_tree host APT |
| QEMU track A | ~87 | NOT RUN | Unpack/Rust/Ninja included; A_RC=0 |
| Nested fat guest preparation | 207 | NOT RUN | Ubuntu 24.04 guest; separate from the host cache |
| Musl build | 23.30 | NOT RUN | V7 then failed at an adapter output-path mismatch, since fixed |
| Dependency-cache restore | absent | FAIL before download | V8 reports download/extract/install and total seconds |
| Payload entry → READY | not reached | NOT RUN | Target approximately 900 s; measured independently of provider boot |

MEASURED on 2026-10-23, run `v7-kvm-20261023`: the saved archive snapshot contained 327 complete .deb files / 1,355,662,990 bytes, including the image baseline; the transaction-only count/size was not preserved. Release-file probes returned 346,021 / 1,849,679 / 1,336,943 B/s for archive / Azure / edge; Azure was chosen. These small-file probes do not establish bulk throughput.

CODE READ: `provision_box.sh --packages ci` lists the required build/driver/initrd tools; the default manual `full` profile additionally keeps clang, lld and libssl-dev. The reviewed Cargo locks have no OpenSSL/native-tls dependency, and the required flows use GCC; the firmware linker comes with Rust. `--packages bench` shares the existing QEMU/guest-image dependency list with the producer. The payload forces `ci` and excludes APT recommendations. No tested project binary is cached: every checkout rebuilds its exact SHA.

MEASURED on 2026-10-23, run `v8-cache-build-20261023`, [producer evidence](../../traces/vast_setup_v8_20261018/cache_build.json): an authenticated Ubuntu 22.04 empty-dpkg-state APT plan resolved 255 packages / 158,286,760 bytes. Six concurrent HTTPS downloads took 6.786 s; the producer took 272.302 s including per-package signed-index metadata lookups. Concurrent 8 MiB / maximum 10 s bulk probes chose edge (0.124457 s), versus archive (1.044052 s) and Azure (timeout, no bytes). This was the producer machine, not the acceptance host.

MEASURED on 2026-10-23, run `v8-cache-offline-20261023`: a fresh Ubuntu 22.04 container with networking disabled installed that closure in 14.330 s after a 0.040 s local-index update. This is a local installation check, not the rental setup measurement. [Published cache evidence](../../traces/vast_setup_v8_20261018/cache_publication.json) records both downloaded-and-verified release assets: 158,145,005-byte .deb archive and 7,888,026-byte Cargo vendor archive (54 crate directories; root and firmware locks).

CODE READ: the one-off owner-invoked producer is `bash scripts/ci/build_setup_cache.sh /absolute/new/output setup-cache-<tag>`; it uses the digest-pinned Ubuntu 22.04 producer Dockerfile, signed APT SHA256 metadata, bounded six-way prefetch and `cargo vendor --locked --sync firmware/kf-gop/Cargo.toml`. Review the generated lock, publish the two assets to the private store with `gh release create --prerelease --latest=false`, and commit the reviewed lock. This flow has no PR trigger, rental or private-store credential inside the producer container.

CODE READ: the production tiny onstart remains unchanged. Repo-side onstart installs `kf-box-blob-url`; the dependency consumer requests two run-scoped locators, keeps each signed URL out of logs/argv, permits only fixed GitHub CDN HTTPS hosts without redirects or credentials, and checks exact size plus SHA256 before extraction. Extraction rejects traversal, links, special files and expansion beyond 8 GB. Existing extraction directories are preserved on refusal. No store PAT, Vast key, App key or owner stop token reaches the box.

CODE READ: `setup-cache.lock.json` binds intended KVM image, Ubuntu 22.04/x86_64, package policy and both Cargo locks. OS/architecture checks are compatibility checks, not host attestation. Each asset is below 2 GB. The box is untrusted; approved code plus reviewed hashes, not a claim of provider trust, establish the chosen cache contents. The project remains an untrusted GPU tripwire.

CODE READ: at boot three bounded bulk probes record network conditions; restored host packages actually come from the local file repository. APT disables all global source fragments and recommendations, uses an isolated list directory, host queues and pipeline depth 10. Cargo replaces crates.io with the hash-checked vendor directory and runs offline. Ubuntu 24.04 guest APT and driver/toolchain/image downloads keep their existing flows and are timed separately. Missing, stale or corrupt cache data produces structured FAIL/setup_cache rather than falling back to unbounded network installs.

INFERRED: avoiding the two host dependency downloads should remove the dominant V7 delay. Falsifier for the fresh lease: dependency restore plus provision_full to READY still exceeds about 900 s, or any required build dependency is absent. The single V8 lease must also show unattended first reporting and MANUAL retention beyond the saved GRACE deadline; no source restart is permitted for those claims.

MEASURED on 2026-10-23, run `v8-gpu-free-20261023`, [preflight evidence](../../traces/vast_setup_v8_20261018/preflight.json): 282 GPU-free tests pass. The standalone EXPECTED RED commits precede the implementation (package policy, forced CI profile, cache identity/extraction, APT metadata/probes, blob-helper port, entry order, recommendations, epoch filenames and APT hook). Explicit fixture migration adds the new helper to both canonical copy lists; production inline-onstart expectations are not restored. The new cache/policy and existing payload/environment/diagnostic mutation ledgers name at least two failing tests per mutant. Canonical 60-case mutation results are recorded separately under their original rule.

MEASURED on 2026-10-23, run `v8-kvm-20261023`: no fresh-lease package/cache/build durations exist because the first blob mint was refused while Vast still reported created. The provider loader and original-grace MANUAL retention were demonstrated; the setup-speed hypothesis remains unverified on a rental. The subsequent event-based repo fix has GPU-free tests only. A new acceptance lease requires owner authorisation; the single V8 lease is destroyed.

CODE READ: deployment of this payload uses the already-shipped blob route. Set Worker var `BLOB_REPO=reindertpelsma/win-gpu-image-private-store` and Worker secret `BLOB_GITHUB_TOKEN` to a contents-read credential for that store. If `BLOB_ALLOWED_TAG_PREFIXES` is configured, include `setup-cache-` in its JSON list. These settings stay in the Worker; the box receives only its existing run capability. No Worker bundle rebuild or CI repository secret is required for the cache.

CODE READ (owner-approved follow-up, 2026-10-23): the box blob helper keeps bounded GET retries for the provider's created-to-running lag: curl exponential backoff, at most 8 retries, 20 seconds per request, and a 299-second hard process deadline for the whole operation. The Worker serves `/blob-url` only after its own Vast inspection reports running; this requirement is unchanged. The ACK pipe remains the primary readiness event. Reporter 401/403/404 handling remains terminal; only the blob GET helper has this bounded retry policy.

CODE READ: cache restoration prepares the file repository and Cargo vendor source; APT update/install remains in the existing provision scripts, after their unattended-upgrade masking and dpkg lock guard. The local-cache shim forces their host APT calls to the reviewed file repository. The separate offline-install check above exercises that repository without bypassing the real provisioner's guard.

## Fresh acceptance after V8

MEASURED on 2026-10-23, run `v8-fresh-kvm-20261023`, source commit 7e0fc2fcb8d8b362cb4c872ab8e03f0c682222c9: [fresh evidence](../../traces/vast_setup_v8_fresh_20261018/LIVE_RUN.md) records the following result (one lease, no spread).

| Phase | Fresh result | Interpretation |
|---|---:|---|
| Rent-start → first report | 296.097 s | Unattended; provider boot differs from V8 |
| .deb fetch/verify/extract | 6.756 s | 158,145,005 compressed bytes |
| Vendor fetch/verify/extract | 2.720 s | 7,888,026 compressed bytes |
| Entire cache restoration | 20.684 s | Includes bounded bulk probes and ACK wait |
| Fresh package install | NOT TIMED | Existing guarded provisioners retain APT work |
| Bench READY | NOT REACHED | Shell HOME was unset at provision_box line232 |
| CUDA/merge/thin/fat suites | NOT RUN | First provision red stopped later stages |

CODE READ: repo-side onstart now restores a missing/empty standard HOME from the current OS account, retaining explicit values. This post-lease fix has tests only; it was not applied to the immutable rented checkout. No kayfabe crate behavior, Worker rule or deadline changed.

## Cloud-init replay before the next acceptance lease

MEASURED on 2026-10-23, run `v8-cloud-init-replay-20261023`: two fresh Ubuntu 22.04 containers
with external networking disabled drove the actual entry/cache/provision_box/provision_full,
host driver, bench tree, CUDA ladder and merge-bar orchestration. The root non-login shell had
no HOME, USER, LOGNAME or TTY; systemd was absent. KF values were only in /etc/environment.
The replay uses explicit GPU/kernel/VM/compiler and deep suite fixtures; it is not a GPU verdict.
Both scenarios passed their assertions: complete fixture PASS in 73.729 s; injected provisioning
failure after the 575.51.03-to-580.159.04 swap in 55.325 s, with fresh hardware in the posted FAIL
and no later CUDA stage. APT installed real reviewed packages offline. [Replay evidence](../../traces/vast_cloud_init_20261018/replay.json).

CODE READ: the fresh-image replay exposed undeclared ssh-keygen/SSH-client dependencies.
The shared required package set now includes openssh-client, and the skip-APT probe checks ssh,
scp and ssh-keygen. Standalone EXPECTED RED commits 4238e09fd and c1b74a3d8 cover both the
package declaration and the skipped-install defect. No Worker code/default/deadline changed.

MEASURED on 2026-10-23, run `v8-cloudinit-cache-publish-20261023`: cache v2 contains 259 .deb
packages / 159,395,796 raw bytes; its compressed .deb asset is 159,253,208 bytes. Vendor remains
7,888,026 compressed bytes and both Cargo locks are unchanged. Both published assets were
re-downloaded and hash checked, including with the Worker's read-only store credential in run
`v8-cloudinit-cache-ro-20261023`. [Publication](../../traces/vast_cloud_init_20261018/cache_publication.json),
[read-only check](../../traces/vast_cloud_init_20261018/cache_ro.json), [producer](../../traces/vast_cloud_init_20261018/cache_build.json).

## Fresh lease after the cloud-init replay

MEASURED on 2026-10-23, run `v8-cloudinit-proof-kvm-20261023`, source d196c055a:
first report121.775s; .deb fetch/hash/extract7.027s; vendor2.361s; full cache20.346s;
provision340.059s to READY580.159.04. Cache plus provision sums to360.405s, excluding small inter-stage overhead.
INFERRED from this run: setup meets the roughly900s target.
Host CUDA4/4, kf tests3749/0, gates9/9+gate10 and kf3 build passed. Raw defer-liveness returned
wrapper rc6 after six passing arms; remaining raw/thin/census/fat work stopped. Runner bound
FAIL; this does not establish the underlying client/Xid cause. SSH diagnostics were denied,
so separate APT timing and independent artifact/hardware snapshots remain unavailable.
The final failure hardware was580.159.04 after initial575.51.03; the checkout's Cargo paths
were used by the executed clients. Exact teardown is complete; inferred cost upper0.242045705 USD,
not an invoice, from the recorded price/lifetime and full transfer reserve in this run.
[Full result, limits and remaining gaps](../../traces/vast_cloud_init_20261018/LIVE_RUN.md).
