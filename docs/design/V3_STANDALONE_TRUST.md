# Standalone trust: what a repo-defined download is anchored by, and what a third party has to trust

STATUS: LIVE, 2026-11-17 (owner rules recorded in `docs/OWNER_RULINGS.md` section AX; step 1 of the plan is built and tested GPU-free, no box and no Vast rental ran any of it; the attestation anchor is a stub that refuses by name, and the CI workflow that would build and attest what only we can build is NOT built). Code: `scripts/ci/kf_trust.py` (the helper), `scripts/ci/trust_manifest.json` (the list), `scripts/ci/trust_lint.py` (the gate). Tests: `scripts/ci/test_kf_trust.py`, `scripts/ci/test_trust_lint.py`; mutants: `scripts/ci/kf_trust_mutants.py`, `scripts/ci/trust_lint_mutants.py` (two named killers each, in `ci.yml`). User-visible behaviour: `docs/howto/run-windows-images.md`, section "What the repository downloads, and what you have to trust".

Labels: **MEASURED** = fetched from the upstream host on 2026-11-17 and hashed (the bytes were streamed or written to a scratch directory, never run). **READ** = read in the tree. **INFERRED** = reasoning, not measured. **NOT DONE** = named gaps. The audit this follows is the coordinator's `STANDALONE_TRUST_AUDIT_20261117` (it is not in the repository); its row numbers are used below.

## 1. The rule

Two actors (OWNER_RULINGS section AX.1):

| Actor | Trust | What a pin means |
|---|---|---|
| **Operator**: runs their own Worker, boxes and object store | implicit, in their own storage and pipeline | deployment data: it guards against staleness, corruption and accidents; not a security anchor |
| **Third party**: dislikes opaque blobs | none in us or in any host we run | an opaque pin of something we built or host is **not** an anchor |

A standalone run's default is **trust git only**. What the repository downloads by itself must be checked against an anchor that does not depend on us. Lifting that is explicit and recorded. A path or URL the user passes is always trusted and recorded.

## 2. The anchors

A repo-defined artifact lists its anchors in the manifest; the helper checks every anchor it can evaluate, and **any disagreement refuses before the artifact is fetched** (the pin says X, the upstream checksum says Y: `TRUST_PIN_MISMATCH`).

| Class | What it is | Where it is checked | Notes |
|---|---|---|---|
| `official-checksum-url` | a checksum document fetched at run time from the UPSTREAM origin | `kf_trust._expectations` | refused when the URL is on a host we run (`TRUST_SAME_ORIGIN`); a document that moves to another host is refused. Same-origin with the artifact adds little (INFERRED, audit section 0b.2); it catches a changed file whose pin was not updated. |
| `upstream-pin` | a pin in git of an upstream-authored identifier anyone can re-check: a commit id, a vendor-published checksum (NVIDIA `.sha256sum`, Blender `.sha256`, PyPI digest, GitHub asset digest), a registry digest, or the sha256 of a source or of an upstream binary upstream still serves byte-identically | the same function; git pins in `resolve_git` | every pin names `pin_source_url` (where anyone re-checks it), `scope` (`source` or `upstream-artifact`), `status` (`MEASURED` with its date, or `CARRIED` from a named file) |
| `attestation` | a GitHub attestation, verified offline | `verify_attestation` | **NOT BUILT.** It always raises `TRUST_ATTEST_NOT_IMPLEMENTED`. If it is an artifact's only anchor the artifact is refused; beside another anchor it is skipped by name in the record. |
| `git-sha256-of-our-own-artifact` | a sha256 in git of something we build or host | never | not an anchor: `trust_lint.py` fails on it (`FORBIDDEN_ANCHOR`) and the helper refuses it (`TRUST_ANCHOR_FORBIDDEN`) |
| `user` | what the user passes | `_user_object` | trusted; sha256 and size recorded; `--expect-sha256` enforced in every mode |
| `operator-pin` | the operator's own sha256 (instruction or pins file) | strict mode only | deployment data, recorded as such; ignored in the default mode |
| `unverified-lifted` | the result of `--trust-lift REASON` / `KF_TRUST_LIFT` | `_lifted` | the computed sha256 is recorded, the result is **non-gating**, `KF_TRUST_LIFTED` is printed; never implied |

**Pending owner confirmation.** The owner agreed to a pin in git for SOURCE inputs. The helper also accepts the sha256 of an upstream BINARY that upstream still serves byte-identically (route 3 for a binary); it marks such an entry `pending-owner-confirmation` when no vendor checksum stands behind the pin. `python3 scripts/ci/trust_lint.py --pending` lists them. On 2026-11-17: `blender-open-data-launcher-3.3.0`, `cuda-keyring-1.1-1-ubuntu2204`, `cuda-keyring-1.1-1-ubuntu2404`, `geekbench-6.4.0-linux`, `geekbench-6.5.0-linux`, `geekbench-7.0.0-linux`, `nvidia-linux-run-570.148.08` (only on `tesla/`, no `.sha256sum`), `nvidia-windows-580.88-exe`, `virtio-win-0.1.302-iso`, `virtualgl-3.1.3-amd64-deb`, `windows-11-ltsc-2024-eval-iso`. If the owner rules against it these lose their anchor: the run then needs the user's own file or the explicit lift. Entries with a vendor-published checksum behind the pin (the other 14 NVIDIA `.run` versions, Blender, vkpeak, rustup-init, the Ubuntu image, websockify, OpenSSH) and every source or commit pin are not affected.

## 3. The helper

`scripts/ci/kf_trust.py` (stdlib only, works under `python3 -I`, importable from `scripts/` and usable from the payload).

```
resolve(name, out, *, user_source=None, expect_sha256=None, mode=None, lift=None, operator_pin=None, transport=None, evidence_dir=None)
resolve_git(name, dest, *, user_source=None, mode=None, lift=None)      verify_git(name, dir)
```

The audit's result labels map as: `route3-pin` = `upstream-pin`, `route1-upstream-sum` = `official-checksum-url`. The result is a dict with `name, path, sha256, size, anchor, detail, origin, source, mode, gating` (and `commit` for git, `lift_reason` when lifted). It is written as one line of `trust.jsonl` (`KF_TRUST_EVIDENCE_DIR` or `--evidence-dir`), with the date and the sha256 of the manifest that judged it, and printed as `KF_TRUST ...` on **stderr** (stdout belongs to the caller: `qemu_source.py` prints the path there). The origin is recorded without credentials, query or fragment (a presigned URL's signature never reaches the evidence); a user path is recorded as its file name.

| Mode | Set by | Meaning |
|---|---|---|
| `trust-git-only` (default) | nothing | a repo-defined artifact needs an anchor; a user object is trusted and recorded; our git pin is compared and noted for a user object, never enforced |
| `strict` | `--mode strict`, `KF_TRUST_MODE=strict` | the operator's mode: a user object is also held to the pin the manifest (or `--operator-pin`) holds, and `operator-pin` is an anchor |

Transfers: a pinned download runs `scripts/ci/kf_fetch.py` (parallel, fail closed, https only except a loopback server for the tests); documents and unpinned (user or lifted) downloads use urllib with a byte bound. **A failed transfer leaves nothing beside the output path** (kf_fetch's own resume state is deleted); the bytes appear at the output path only after every check passed. The transport is not trusted to have enforced the pin: `resolve` compares what the transport reports.

Refusals, all named `TRUST_<NAME>` on stderr, exit status 3 unless noted: `NO_ANCHOR`, `PIN_MISMATCH`, `UPSTREAM_SUM_MISMATCH`, `UPSTREAM_SUM_BAD_DOC`, `SAME_ORIGIN`, `ANCHOR_FORBIDDEN`, `ATTEST_NOT_IMPLEMENTED`, `EXPECT_MISMATCH` (the user's own, never liftable); `TRANSPORT` (exit 1), `UNKNOWN_ARTIFACT` (exit 4: no manifest entry; a user object needs none), `USAGE`, `BAD_MODE`, `BAD_LIFT`, `BAD_MANIFEST` (exit 2). `python3 scripts/ci/kf_trust.py check-evidence trust.jsonl` exits 3 when any line is `unverified-lifted`: the run report is meant to refuse a gating result with such a line (**NOT DONE**: `vast_report.py` does not call it yet).

## 4. The manifest and the lint

`scripts/ci/trust_manifest.json` (schema `kayfabe.trust-manifest/1`): 57 artifacts on 2026-11-17 (38 routed through the helper, 19 `native`: the file keeps its own fail-closed pin check and the manifest lists it), and `unrouted`: 56 entries for 181 download-primitive lines in scripts that do not go through the helper yet. Each artifact has `row`, `kind` (`file`, `git`, `oci-image`), `origin`, `runs` (where the bytes execute), `consumers`, `anchors` and optionally `pin_files` (strings that file must still hold: a pin that moved away from its code is `PIN_DRIFT`).

`scripts/ci/trust_lint.py` (in `ci.yml`, stable job; ignored by the ladder like the other gates) fails on: a forbidden anchor, an artifact without an anchor, an invalid manifest (same-origin checksum, a pin whose source is ours, missing pin fields), a missing consumer, a helper-routed consumer that does not go through `kf_trust` or still spells the origin URL, pin drift, an image digest that left its Dockerfile `FROM`, an unpinned Dockerfile `FROM`, and **any download primitive (`curl`, `wget`, `urlopen`, `pip install`, `git clone/fetch`, ...) in `scripts/` or `tools/` that is neither routed nor on the ratchet**. The `unrouted` list is the one committed baseline of this change; it was unavoidable (181 lines in 80 files). It is a ratchet: each entry names a class and a reason, the count can only go down (a lower real count is `STALE_UNROUTED` until the entry is lowered), and a new download in a listed file is a finding. Nothing here is re-recorded to pass a finding.

## 5. What changed, by audit row

| Row | Change | Evidence |
|---|---|---|
| 16 NVIDIA Linux `.run` (host, root) | `provision_host_driver.sh`, `provision_bench_tree.sh`, `box/windows_lane.sh`, `drivermatrix/stage_guest_driver.sh` fetch it through the helper. 15 versions pinned: NVIDIA's `.run.sha256sum` plus the same value as the vendor-published pin. The `tesla/` fallback is gone: 570.148.08 (404 on `XFree86/`) carries its `tesla/` URL. A version the manifest does not list is `TRUST_UNKNOWN_ARTIFACT` (the stage falls to its deb fallback as a 404 always did). `RUN_URL` is a user input. | **MEASURED** 2026-11-17: all 15 streamed from `us.download.nvidia.com`; 14 of 15 `.sha256sum` documents equal the streamed hash; the 15th has none |
| 3 rustup | `vast_box_main.sh` and `provision_box.sh` fetch `rustup-init` 1.29.3 and check it against `static.rust-lang.org/.../rustup-init.sha256` and the pin; `provision_box.sh` installs it after the clone (the helper comes with the tree) | **MEASURED**: 21347088 bytes, sha256 equals the official `.sha256` |
| 29 ogkm clone | `stage_guest_driver.sh`: `git-checkout ogkm-<v>`, HEAD must equal the pinned commit; a cached tree at another commit is removed and fetched again. 15 tags pinned. | **MEASURED**: `git ls-remote` (peeled tag commits) |
| 24 VirtualGL | `provision_guest_gfx.sh` fetches the `.deb` on the host (pin), pipes it into the guest | **MEASURED**; no upstream checksum: pending confirmation |
| 25 Blender, Blender Open Data, vkpeak, Geekbench | `gfxset/provision.sh` fetches on the host and pipes into the image; `apps/setup_side.sh` pins Geekbench like Blender through `pins.sh`. Blender: official `.sha256` + pin; vkpeak: GitHub asset digest; Geekbench 7.0.1 is 404 upstream and was dropped from the list. | **MEASURED** |
| 20 Dockerfile | `scripts/kfml/Dockerfile`: `FROM nvidia/cuda:12.8.1-devel-ubuntu24.04@sha256:520292db...` | **MEASURED**: `Docker-Content-Digest` of the tag on registry-1.docker.io, 2026-11-17. A tag can be re-pushed; a digest cannot. |
| 30 SPEC | `specviewperf_stage.sh` + `stream_url.py`: a user's URL file is trusted and its sha256 and size recorded (`trust.jsonl`, `SPEC_NOTE your archive: ...`); our git pin is only compared (`pin=match\|differs`) unless `KF_TRUST_MODE=strict`; `KF_SPEC_EXPECT_SHA256` is the user's own enforced pin. The Worker's `/bench-data` route keeps the pin as before. | GPU-free stage tests (`UserUrlFile`), 8 tests |
| 4 QEMU | `qemu_source.py` goes through the helper: same refusals, same messages, the pin file stays the single source (`trust_manifest.json` holds the same value; the lint checks it). `--from-file` is **still held to the pin** (behaviour kept; see section 7). | **MEASURED**: re-hashed from `download.qemu.org`, equals `qemu.pin` |
| 17 kfml CUDA redist | six `native` entries (nvcc, cudart, cccl for cu126 and cu128): sha256 and size from NVIDIA's own `redistrib_12.6.3.json` / `redistrib_12.8.1.json` (vendor-published), equal to `scripts/kfml/pins.json`; the lint holds the two together | **MEASURED** 2026-11-17: the vendor manifests, not the tarballs, were read |
| 5, 9, 10, 11, 12, 13, 14, 15, 27 | listed in the manifest as `native` entries with `pin_files`: their code keeps its own check and the lint holds the value in the code to the manifest. 5, 10 (noVNC), 11, 14 and 9 re-measured today and equal the pins; 12, 13 and 15 are `CARRIED` (not re-measured: 5 GB, 0.9 GB, 0.9 GB). cuda-keyring: re-measured. | |

**Mirror refinement.** `OWNER_RULINGS` section AX.4: a user-passed mirror, URL or path for a repo-defined artifact is a user input. The helper does exactly that (`resolve(..., user_source=...)`), and the scripts above take `RUN_URL`, `VGL_URL`, `BLENDER_URL`, `VKPEAK_URL`, `BOD_URL` and `GB_URLS` as user sources.

## 6. Behaviour changes a user will notice

1. A host driver version that `trust_manifest.json` does not list is refused (`TRUST_UNKNOWN_ARTIFACT`) instead of being downloaded on TLS alone. Add the version to the manifest, pass `RUN_URL=<your url or path>` (trusted, recorded) or set `KF_TRUST_LIFT=<reason>` (recorded, non-gating).
2. `provision_box.sh` needs a branch that carries `scripts/ci/kf_trust.py` (it says so by name); its default `KAYFABE_BRANCH` of old does not.
3. `gfxset/provision.sh` and `provision_guest_gfx.sh` download on the host and pipe into the guest instead of downloading in the guest.
4. A cached installer or a cached ogkm tree is checked against the anchors; one that does not match is replaced.
5. `KF_TRUST` lines appear on stderr of the scripts that use the helper.

## 7. NOT DONE (named)

- **2026-11-20: `python-3.12.10-source` (the private Python of the open ML rows, `scripts/apps/mlrows/private_python.py`) has no manifest entry yet.** The tarball is fetched only through this helper, so until the entry exists `kf_trust.py fetch` says `TRUST_UNKNOWN_ARTIFACT` and the build refuses by name. The anchor it can hold is an `upstream-pin` of the sha256 of the SOURCE (python.org lists an MD5, a GPG signature and a Sigstore bundle, none of which this helper evaluates), which needs one measurement on a machine with network: `private_python.py pin-entry --tarball FILE --md5 <the release page's MD5>` prints the entry.
- **The attestation route and everything that needs a CI workflow** (audit steps 5 to 7): rows 21 and 22 (the public bundle and the test-assets ISO are our builds: a FAIL for a third party today), 34 (deqp-vk, not found in the tree) and the optional setup-cache attestation. `trust_manifest.json` lists the consumers as `unrouted` with class `planned`.
- **Row 8 (setup cache) and the `kfhost` `borrowed` profile** (`scripts/kfhost/downloads.py` still treats a git-pinned `(url, sha256)` of our setup cache as an anchor; the audit asked for plain apt and crates.io for a third party). Not touched.
- **Row 26 (torch) and the kfml wheels (rows 17 to 19).** The audit says kfml's wheels have a route-3 anchor; the tree says otherwise: `scripts/kfml/requirements*.lock.txt` read "HASHES TODO" and `kfml setup` refuses without `--allow-unhashed`. So the wheel closure has **no anchor today** (READ), and `setup_side.sh` / `provision_guest_llm.sh` install `torch==2.6.0` by version only. A hashed closure per Python version is M effort and was skipped. The kfml CUDA redist is listed (row 17, above); the two datasets (row 19: a `raw.githubusercontent.com/.../master/...` file and a CIFAR tarball from cs.toronto.edu) are pinned in `pins.json` and not listed in the manifest; `setup_venv.py` and `datasets.py` are `unrouted`, class `native-route3`.
- **Rows 6, 7, 23, 28, 31 to 33 and the apps image recipe** were not ported; the BtbN `latest` ffmpeg (a floating URL), the floating `noble/current` cloud image and the CUDA cudart URL in `provision_bench_tree.sh`, the CUDA-repo deb index scrape in `stage_guest_driver.sh`, glmark2 at a tag with no commit check, `provision_bare_metal.sh` (it pins the old branch `w749-fable-legb`, which carries no helper) and the Blender Open Data launcher's own downloads inside the guest are `unrouted` with their reasons.
- **Rows 9 and 10** (GitHub archive tarballs are not promised byte-stable) are listed, not changed to commit-pinned clones.
- **`qemu_source.py --from-file`** is still held to the pin (the owner's section AR wording). Under AX.4 it is a user path; making it a plain user input is a one-line change and an owner decision.
- The run report does not yet refuse a gating result with a lifted line (`kf_trust.py check-evidence` exists).
- Signature upgrades (QEMU `.sig`, Ubuntu `SHA256SUMS.gpg`, Authenticode): audit step 8.

## 8. The correction to the audit's numbering

The audit cites "§AW.4" for the supply-chain sentence. In the tree it is §AW.2 of `OWNER_RULINGS.md` and rule 4 of `V3_PINS_ARE_DEPLOYMENT_DATA.md`; both carry the refinement above the text it refines, in the sentence itself.

## 9. Tests

`test_kf_trust.py` (131 tests): each anchor class, the default mode, the explicit lift, user input, strict mode, evidence (no presigned query, no credentials, file name only), the published file mode, git commit pins on local repositories, the real transport against a loopback server (including a relative output path, which the first live run against real hosts broke), the CLI exit codes. `test_trust_lint.py` (68 tests): fixture trees for every finding plus the committed tree. `kf_trust_mutants.py` (48 mutants) and `trust_lint_mutants.py` (28 mutants): one exact-once replacement each, both named killers must fail. Changed code kept its own suites green: `test_qemu_source.py`, `test_apps_pins.py`, `test_spec_pc_knobs.py` (+ 6 tests, + 5 mutants in `stream_url_mutants.py`), `test_vast_spec_stage.py` (+ 8 tests). The whole `scripts/ci` discovery ran 5866 tests: green except two 5-second subprocess timeouts of `test_vast_setup_packages` that passed when run alone (the machine was busy; INFERRED unrelated).

Live check (MEASURED 2026-11-17, from a scratch directory, against the real hosts): `kf_trust.py fetch` of `rustup-init-1.29.3`, `nvidia-linux-run-580.159.04` (397162548 bytes in 7.5 s, official checksum and pin both agree), `virtualgl-3.1.3-amd64-deb` (GitHub redirect), `blender-open-data-launcher-3.3.0` and `vkpeak-20250531-ubuntu` passed; `git-checkout ogkm-580.159.04` ended at the pinned commit `b81d58ee...`; a one-byte-longer copy was refused `TRUST_UPSTREAM_SUM_MISMATCH` (exit 3), an unlisted version `TRUST_UNKNOWN_ARTIFACT` (exit 4), a bad mode `TRUST_BAD_MODE` (exit 2).
