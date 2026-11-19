# Vast box scripts (repo side of the CI broker)
STATUS: LIVE, 2026-11-16 (kfml payload stage, `--only kfml`, GPU-free tests plus one end-to-end run on a CUDA container); earlier: LIVE, 2026-11-15 (propimage_apps payload stage, `--only propimage_apps`, GPU-free tests only, no box has run it; box downloader `box_downloader.py`, built and tested GPU-free, run on no box and wired into no stage; payload `--instruction FILE`, `--only/--skip/--dry-run`: GPU-free tests only, no box has run them; the rest of this page is as of 2026-11-06) (the failed-run evidence bundle, `vast_evidence.py`, is new and has run on no box); box protocol 7 (ported from protocol 4/5 on branch
claude/vast-payload-p7-20261024). MEASURED 2026-10-25: a real Vast box booted with the
protocol-7 setup but had not yet reported to the Worker, so protocol 7 has no end-to-end run on a
real box yet. The evidence below is for the pre-port payload. MEASURED: run `v9-xid-kvm-20261023`, source
`2c908a0e8`: bound PASS, host/fat CUDA4/4 each, tests3749/0, gates9/9+10,
raw30/30, thin30/30 and census. [Latest evidence and gaps](../../traces/vast_xid_v9_20261018/LIVE_RUN.md).
The earlier V8 run stopped at raw defer-liveness wrapper rc6; that finding is superseded by
S4 exact-source admission and the complete V9 result. Its journal was not retained for replay.
The Worker's tiny onstart clones the repo, checks out `KF_SHA`, execs `scripts/ci/vast_onstart.sh`.
Per-run data is `KF_*` environment variables. Files: `vast_onstart.sh` (tools, env check, root-only
env file `/run/kf-ci/env`, helpers, reporter, payload under `timeout`); `vast_report.py` (stdlib:
`check-env [--write F]`, `daemon --log L --done D --result R`); `kf-box-kill`, `kf-box-blob-url`;
`kf_dns_fallback.sh` (2026-11-11: run first by `vast_onstart.sh` and before `apt-get` in the GitHub jobs; if archive.ubuntu.com does not resolve it points the resolver at 1.1.1.1 and 9.9.9.9 and prints one `KF_DNS_FALLBACK used=<0|1> [ok=<0|1>]` line; never fails the run; tests `test_vast_setup_dns.py`);
`test_vast_box.py`, `vast_box_mutants.py`; `vast_evidence.py` (2026-11-06: the failed-run evidence bundle, run by `vast_onstart.sh` after the payload; [VAST_PAYLOAD](VAST_PAYLOAD.md), "Evidence bundle"); `vast_run_report.py` (2026-11-07: the structured run report, run by `vast_onstart.sh` after the payload when the Worker sets `KF_RUN_REPORT=1`; [VAST_PAYLOAD](VAST_PAYLOAD.md), "Run report"); `fetch_evidence.py` (coordinator tool, not CI). There is no hold helper or login hook (owner ruling
2026-10-23 19:10; the Worker has no hold route). The box sends the Worker-injected Cloudflare
Access pair (`KF_ACCESS_ID`, `KF_ACCESS_SECRET`) as `CF-Access-Client-Id`/`-Secret` on every
broker call when both are set (owner ruling 2026-10-23 21:45); the payload process does not get
it, but the box code can read the root-only env file ([threat model](../../ci/vast-broker/docs/THREAT_MODEL.md)).

Env (protocol 7, declared by `BOX_PROTOCOL` in `vast_report.py`; the Worker reads that line at
the tested SHA before renting). Required: `KF_PROTOCOL`, `KF_BROKER_URL` (https origin + secret
path, no trailing slash), `KF_RUN_ID`, `KF_RUN_TOKEN` (a compact HS256 JWT; the box only checks
its shape), `KF_NONCE` (32 hex), `KF_SHA` (40), `KF_RUNNER_NONCE` (64), `KF_REPO`,
`KF_SOFT_DEADLINE` (epoch s), `KF_HEARTBEAT_SECONDS`, `KF_GRACE_FAIL`. Checked if present:
`KF_BOOT_DEADLINE`, `KF_HARD_DEADLINE`, `KF_GRACE_PASS`, `KF_ACCESS_ID`, `KF_ACCESS_SECRET`. A
violation posts a log naming variables (never values) and, if nonce/sha/runner nonce are valid, a
FAIL result `env_contract`; the payload does not run.

Payload: `bash scripts/ci/vast_payload.sh <sha> <nonce>`, cwd = checkout, stdin closed, output to
`/workspace/kf-ci-output.log` (offline backup). It gets `KF_SOFT_DEADLINE_MS` and loses the token,
broker URL, run id, runner nonce and Access pair. It writes `/workspace/kf-result.json`:
`{verdict, suites:[{name,pass,fail,skip}], hardware?, diagnostics?}`, hardware = `{gpu_name, compute_cap (int,
8.6 -> 860), driver_version}`. Nonce/sha/runner nonce come from the environment; a conflicting
value in the file makes it invalid. PASS is withheld (FAIL + failing row) on non-zero exit, skips,
zero passes, unknown hardware or compute_cap < 750. Soft deadline: TERM, KILL after 30 s; result is
FAIL with the suites already written plus `timeout`. Absent payload: FAIL `payload_missing`; absent
or invalid result file: FAIL `result_missing`/`result_invalid`. Exit text never makes a PASS.

kfroot and rootless Windows (2026-11-15, OWNER_RULINGS section AR item 6; GPU-free tests and one hand run of the real sudo in a sandbox, NOT run on a box):
`kfroot.py` installs the explicit root step (`/usr/local/bin/kfroot` = `exec sudo -n -E` on a root-owned wrapper that is the only sudoers entry; the
sudoers file is `visudo -c` checked before it is installed), bootstraps the box for an unprivileged `ci` user in group `kvm` (device nodes, memlock,
`/workspace`, GPU lock, sudo log), starts a command dropped (`run`: every capability set empty, `NoNewPrivs=1`), and asserts the result from `/proc`
and the sudo log (`assert-tree`, `audit-check`). `windows_gate.sh` and `windows_apps.sh` no longer refuse non-root: they check `/dev/kvm`, `/dev/nvidiactl`,
writable directories, cargo and a readable kernel log by name, and the root operations (apt, the swtpm AppArmor rule, `dmesg` under
`kernel.dmesg_restrict=1`) are `kfroot <cmd>` lines (`scripts/lib/kfroot.sh`). `KF_ROOTLESS=require|record|off` (default record). Design, the
inventory of what really needs root, the measured sudo behaviour and the first-box checklist: [V3_KFROOT_ROOTLESS](../../docs/design/V3_KFROOT_ROOTLESS.md).
Tests `test_kfroot.py`; mutants `kfroot_mutants.py` (two named killers each, in `ci.yml`). `vast_payload.py` does not yet use it.

Instruction file and stage selection (2026-11-15, OWNER_RULINGS section AR, stages S1 and S3; MEASURED by
GPU-free tests only, not on a box): the payload also takes `--instruction FILE` (JSON, at most 64 KiB, schema
`kayfabe.payload-instruction/1`: `ci/vast-broker/planner/schemas/instruction.schema.json`, mirrored byte for byte
in `scripts/kfci/schemas/`). `payload_instruction.py` validates it once (restricted schema interpreter of
`scripts/kfci/planner_assignment.py`, plus cross-field rules) into a frozen `Instruction`; unknown fields, wrong
types, duplicate keys, traversing artifact paths and another schema version are refused (`preflight`). The
KF_* tier variables (`KF_MERGE_WINDOWS`, `KF_TIER`, `KF_WINDOWS_RUNS`, `KF_STAGES`, `KF_SPEC`, `KF_LINUX_APPS`,
`KF_WINAPPS`, `KF_RELEASE_REHEARSAL`, the matrix fields) are ONE adapter, `instruction_from_env`, building the same
object with the same functions as before; with `--instruction` none of them may be set. A release matrix cannot be
spelled in a file. `--only STAGE` and `--skip STAGE` (repeatable) select among the planned stages (a name outside
the plan, or an empty selection, is `preflight`); skipping does not provide a skipped stage's outputs, so a partial
run is local evidence and its result carries `selection`. `--dry-run` prints the `KF_PAYLOAD_TIER` line, the
instruction hash and `KF_PAYLOAD_PLAN stages=... planned=...` and touches nothing (no sha or deadline needed);
`--emit-instruction FILE` writes the env-built instruction. A run from a file records `instruction_sha256` (of the
bytes read) in `kf-result.json`; the reporter validates it and does not forward it. `artifacts` (name, relative
path, sha256) is validated and recorded but not yet consumed (stage S7). The payload still reads no token or broker
URL. Tests `test_payload_instruction.py`; mutants `instruction_mutants.py` (two named killers each, in `ci.yml`).
Not done: the Worker does not write an instruction yet (S2), hard-coded paths remain (S4).

The `kfml` stage (2026-11-17; IN THE DEFAULT PLAN of the middle and large tiers, `--skip kfml` leaves it out; GPU-free tests plus end-to-end runs of `kfml_stage.py` on five Vast CUDA containers, NOT run under the payload on a box):
`kfml_stage.py` runs `scripts/kfml` (flavour plan, setup and pinned fetch-data together, run) on the box's own GPU and prints one `KF_KFML_RESULT {...}` line that `vast_payload.py kfml_rows` grades (PASS; NOT_RUN = skipped row;
anything else fails); the payload prefetches the datasets in the background at its start. Middle runs the five gated rows, large all six. The record is `kfml` in `kf-result.json` (`vast_report.py validate_kfml`). Budget row
`payloadStageResources.kfml` (`ci/vast-broker/defaults.json`), downloads `bandwidth.json` `kfml`. Details in `VAST_PAYLOAD.md`, last sections. Tests `test_kfml_stage.py`; mutants `kfml_stage_mutants.py` (two named killers each, in `ci.yml`).

The `install_flow` stage (2026-11-15, OWNER_RULINGS section AS; GPU-free tests only, NOT run on a box): `install_flow.py` runs
the commands the manual tells users to run (docs/howto/building.md) as a fresh unprivileged user in group kvm with no
capabilities, then boots a Linux guest with the QEMU arguments of docs/howto/attach-to-existing-vm.md and runs `cup8` in it.
The commands are never copied: `doc_steps.py` reads the fenced blocks that carry a `<!-- kf-run: STEP [user=root|unpriv]
[timeout=S] -->` or `<!-- kf-args: STEP -->` line directly above them (convention and refusals in its header), and
`install_flow.PLAN` must list every marked step. Selection: `vast_payload.py --instruction FILE --only install_flow` with a
`large` instruction; it is NOT in the default large plan (`INSTALL_FLOW_IN_LARGE = False`) and joins the release tier by
setting that constant to True in the commit that records its first green box run, adds its `payloadStageResources` row (`ci/vast-broker/defaults.json`, so a Worker bundle change) and takes it out of `MANUAL_ONLY_STAGES`; `profile_resources.py` refuses the flag being True while it is listed there (no schema change). It runs after
`cuda_guest`/`apps` and before the Windows gate, needs 3 hours left, and leaves `install_flow` in `kf-result.json`: verdict,
per-step rc/uid/capabilities, which steps needed root, the substitutions, the QEMU and guest facts and the sha256 of all five
howto documents (the reporter validates the record and does not forward it). The attested-binary hook is
`install_flow.resolve_binary` (returns None today; `--attested-binary` is refused). Tests `test_doc_steps.py`; mutants
`doc_steps_mutants.py` (two named killers each, in `ci.yml`).

CODE READ: optional local `diagnostics` records contain `arm, mode, rc, admitted, text,
sha256, source_bytes, dropped_bytes`. Previews retain head/tail:256 UTF-8 bytes for a clean
arm,2048 for a refused arm; the first refusal is preserved when compacting. Limits:128
records,49152 bytes of JSON including escapes, within the existing65536-byte result-file cap.
Hashes identify the original UTF-8 input text before preview truncation; they confer no trust.
The reporter validates this local field and emits the same existing wire result keys; the
full bounded per-arm context continues through the authenticated log stream. No Worker
schema, bundle, admission rule or verdict comes from diagnostic text. This local-file change
was rehearsed without a new rental; the V9 live proof predates it.

Reporter: `POST <KF_BROKER_URL>/report/<run>`, Bearer token, JSON `{seq,kind,text?,result?}`. A
message keeps its sequence number and bytes until acknowledged (idempotent, no gaps); 401/403/404
stop reporting; 429, 5xx, 409 NOT_READY retry with backoff; 3xx is never followed. Caps: 8 KiB per
post, 4000 bytes per line, 1500 log posts, 3 MB of log bodies, 1950 requests, 1 s between requests.
Heartbeat (`log`, empty text) when no post succeeded for `KF_HEARTBEAT_SECONDS`. Nothing is
stripped (Worker and runner sanitise). The token goes only to `KF_BROKER_URL`, HTTPS, no proxy.

`kf-box-kill [reason]` POSTs `/kill/<run>`: curl `--proto =https --max-redirs 0`, token (and the
Access pair, when both halves are set) on stdin, non-zero unless HTTP 200.
CODE READ (Worker on master: `ci/vast-broker/src/{routes,reports,results,onstart}.js`,
`docs/BOX_PROTOCOL.md`): endpoints, schema, caps, rate 20 per 10 s.
MEASURED: `python3 -m unittest discover -s scripts/ci -p 'test_vast*.py'`,
`scripts/ci/vast_box_mutants.py`; `ci/vast-broker/test/box-scripts-p7.test.js` runs the real
reporter against the in-process Worker.
INFERRED: apt, `/run`, sshd on the real KVM image. The payload is code from the SHA under
test, runs as root and can read the env file.

CODE READ: `kf-box-blob-url <tag> <name>` resolves a private release asset using only the run's
scoped token; the token stays on curl stdin. The payload first restores the dependency cache
from the [reviewed lock](setup-cache.lock.json), then delegates to the existing box provision
and merge bar. [Cache design and timing table](../../docs/design/VAST_SETUP_CACHE_20261023.md).

CODE READ: dependency setup waits for the reporter's first accepted progress acknowledgement
through the internal KF_BROKER_READY_FD pipe. The pipe carries no token; reporter auth
refusals remain terminal. Ordinary payloads retain the offline log path.
The Worker serves blob URLs only once Vast reports running. The box helper retries failed GETs,
including 401 during created, with curl's exponential backoff (8 retries, 20 s per attempt),
under a 299 s hard deadline. The Worker's running-state requirement is unchanged.

CODE READ: host APT runs in the existing guarded provisioners; cache restore prepares files only.
The test runtime is Ubuntu 22.04 with GNU coreutils (`setup-test.Dockerfile`), matching the box.

CODE READ: if cloud-init omits standard HOME or leaves it empty, repo-side onstart restores the
current OS account's home directory. An explicit process HOME is preserved; no Worker var is added.


The `propimage_apps` stage (2026-11-15, owner ruling that the proprietary Windows image's seven apps must PASS per GPU family; GPU-free tests only, NOT run on a box,
and it cannot PASS yet): `propimage_apps.py` fetches the private image through `box_downloader.py` (manifest made from `propimage_apps_pins.json`), builds swtpm 0.10.1 when
the box's is older, unpacks, and runs `scripts/bench/windows/propimage_check.py` in three offline boots (Wukong last, alone, guest framebuffer 8192 MiB), then
records per app the verdict kind, evidence names and sha256, scores, guest TDR/LiveKernelEvent counts and the host Xid lines inside the app's kernel-log window.
Every pin in `propimage_apps_pins.json` is unset (nothing in the repository records the image sha256), so it refuses `PIN_UNSET:image`; and the harness grades
Wukong INCONCLUSIVE (its benchmark menu driver is research tooling in draft PR #417), so it ends `APP_FAIL:wukong` until that is in the harness. Non-AMD hosts are
`NOT_RUN` (`CPU_VENDOR`). Selection: `vast_payload.py --instruction FILE --only propimage_apps` with a `large` instruction; not in the default large plan
(`PROPIMAGE_APPS_IN_LARGE = False`, `MANUAL_ONLY_STAGES`). `propimage_apps.py plan` prints the preflight rows and the boot plan without touching anything;
`propimage_apps.py manifest --out FILE` writes the download manifest (and refuses while pins are unset). The record is `propimage_apps` in `kf-result.json`, validated in
`vast_report.py`. Design, refusal table and first-run checklist: `docs/design/V3_PROPIMAGE_APPS_STAGE.md`. Tests `test_propimage_apps.py`; mutants `propimage_apps_mutants.py`
(two named killers each, in `ci.yml`).
## Standalone run: one presigned-URL file for every private object (2026-11-16)

A box that runs the payload by hand (no Worker, no run token, OWNER_RULINGS section AR item 3) needs the private objects through ONE file,
`/run/kf-ci/download-urls.json` (mode 0600, schema `kayfabe.download-urls/1`: `{"urls": {ref: presigned https URL}}`, extra top-level keys such as the
sizes and sha256 pins of the objects are ignored by the readers). The coordinator, which alone holds the R2 credential, writes it with
`rclone link r2:... --expire <= 12h` per object; the box gets the file and nothing else (no key, token or rclone config).
The readers: `box_downloader.py --urls` (the `propimage_apps` stage: refs `propimage-kfimg-<date>.zip`, `propimage-kf-gop.signed.efi`) and
`scripts/bench/specviewperf/stream_url.py` (SPEC lane, `KF_SPEC_URL_FILE=<file>`; since 2026-11-16 it reads this shape too, object ref
`specviewperf-15.0.0.tar.zst`, and still the PC-run flat map). Mutants: `stream_url_mutants.py`. Every other object of the rehearsal stages is public.

The generic proprietary-image runner (2026-11-16, phase 1; `docs/design/V3_PROPIMAGE_RUNNER.md`, `docs/howto/run-windows-images.md`; GPU-free tests only, run on no box): `propimage_apps.py run --image prebuilt:SOURCE ... [--strict]` (or `--propimage PATH[:sha256]`, or `--images-file FILE`) takes any number of Windows images, each with its applications, through `propimage_images.py`; without an image flag `run` is the stage above. `scripts/bench/windows/propimage_registry.py` and `app_defs/` hold the application definitions (one file per definition set), `propimage_runspec.py` the spec and its named refusals, `propimage_plain.py` the harness for a disk without a state.json, `clocknet_assert.py` the rule that a faked clock needs the network off and a networked guest the real clock (checked in `propimage.py build_argv`, `propimage_run.sh`, `win_vm.sh` and the runner's boot proof). Standalone runs skip what cannot run (exit 0); `--strict` (the instruction's `propimages_strict`) fails on any skip (exit 4). Identity pins (a source's sha256, a `pin:` object's hash in the spec) are enforced only under `--strict`; a standalone run records the computed sha256 and size and `--expect-sha256 NAME=HASH` opts in (`docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md`, `test_propimage_pins.py`; `--pins FILE` names your own deployment's pins). The record `kayfabe.propimage-run/1` is `propimage_run` in `kf-result.json` (`vast_report.py`). Tests `test_clocknet_assert.py`, `test_propimage_runspec.py`, `test_propimage_images.py`; mutants `clocknet_assert_mutants.py` and `propimage_runner_mutants.py` (two named killers each, in `ci.yml`).

## Box downloader (`box_downloader.py`, 2026-11-15; step 3 of [V3_BOX_DOWNLOADER_AND_SUPPLY_CHAIN](../../docs/design/V3_BOX_DOWNLOADER_AND_SUPPLY_CHAIN.md))

MEASURED by GPU-free tests only (loopback HTTP server, `test_box_downloader.py`; mutants `box_downloader_mutants.py`, two named
killers each, in `ci.yml`). Not run on a box, not wired into any stage, no real private R2 object ever fetched. Stdlib only,
Python 3.9+, on top of `kf_fetch.py` (`.unverified` staging, per-chunk Range resume, size + sha256 pinned by the caller,
atomic rename, partial deleted on a size or hash violation). It needs no Worker, no run token and no network credentials: a
normal user needs Python 3 and a manifest.

Standalone example (a local directory pin and an https mirror, nothing else):

```sh
cat > manifest.json <<'JSON'
{"schema": "kayfabe.download-manifest/1", "stages": ["setup"],
 "items": [
  {"name": "tool.tar", "dest": "dl/tool.tar", "sha256": "<64 hex>", "size": 1234567, "needed_by": "setup",
   "sources": [{"kind": "mirror", "url": "https://mirror.example.org/tool.tar"}]}]}
JSON
python3 scripts/ci/box_downloader.py check manifest.json                       # validate, print item count and bytes
python3 scripts/ci/box_downloader.py fetch manifest.json --dest ./out --pin-dir ~/Downloads --limit-bandwidth 50 &
python3 scripts/ci/box_downloader.py wait --dest ./out --deadline 600 tool.tar     # 0 verified, 1 failed, 3 deadline, 4 downloader gone
python3 scripts/ci/box_downloader.py status --dest ./out
python3 scripts/ci/box_downloader.py verify manifest.json --dest ./out             # re-hash every dest
```

`--pin-dir DIR` (repeatable) and `--pin NAME=PATH` use an existing local file when its size and sha256 match (otherwise it is
ignored). Manifest format `kayfabe.download-manifest/1` (at most 1 MiB, strict, validated once in `box_downloader.py`):
`stages` (first-use order) and `items`, each `{name, dest, sha256, size, needed_by, sources[], priority?, local?}`; `dest` is
relative to `--dest`, with no `..` and no symlink on the way; ordering is stage, then priority, then manifest order; sources
are the fallback order and `signed`/`s3` (private) must come before `mirror`. Source kinds: `mirror` (plain https URL,
optional `follow_redirect`), `signed` (a presigned URL looked up by `ref` in `--urls FILE`, JSON `kayfabe.download-urls/1`,
mode 0600, re-read on every attempt) and `s3` (any S3-compatible endpoint, path style; bucket and key in the manifest, the
endpoint and credentials from `KF_S3_ENDPOINT`/`KF_S3_REGION`/`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`
or `--s3-config FILE`, mode 0600; the URL is signed locally with SigV4, 20 minutes). No account or bucket name is in the
code; the repo's own defaults belong in the shipped manifest. `http://` sources need `--allow-http` (tests, trusted LANs).

Defaults (DESIGN values, not measured on any box): 3 items in parallel, 4 streams per item, global cap 125 MB/s
(`--limit-bandwidth MBPS`, 0 = off), per-connection cap off (`--stream-limit`), niced 19 and ionice idle (`--no-nice` off).
Each completed item prints `KF_STEP name=download.<item> secs= bytes= source=<local|present|r2|s3|mirrorN> MBps=`; a failed
one `KF_DOWNLOAD_FAIL name= class=RETRY reason=` (a harness failure, never a guest FAIL). URLs and credentials are never
printed (every line is scrubbed). The bounded status file is `<dest>/.kf-downloader/status.json`. SIGTERM/SIGINT cancels:
exit 5, partial files kept, the next run resumes after re-verifying every saved chunk.

Payload and the token: `box_downloader.py` never reads the run token, the broker URL or the Access pair (a test greps its
source for them). The Worker-aware part is `box_downloader_worker.py resolve MANIFEST --urls-out FILE`, run by the bootstrap
layer that already holds the token (OWNER_RULINGS section AR): it asks `GET <broker>/bench-data/<run>/<name>` for each `signed`
ref, keeps the 302 target, writes the 0600 URL file, and prints no URL. The payload gets only `--urls FILE` (a file, not a
descriptor, because presigned URLs expire after 20 minutes and a refresher must be able to replace the file, and a restarted
downloader must be able to read it again). That keeps the payload from being handed the token; it does not hide it from a
root payload, which can read `/run/kf-ci/env` today. The payload's only hook is `vast_payload.py --dry-run --prefetch
MANIFEST` (prints `KF_PAYLOAD_PREFETCH ...` for the planned stages); without `--dry-run` it is refused. Not done: starting the
downloader from the payload, a stage calling `wait`, apt (design source c), seeders (deferred), the real manifest for the
SPEC and Windows media, disk reservation of unpack space beyond `--reserve-gb`.

Open ML rows (2026-11-17, [design](../../docs/design/V3_OPEN_ML_ROWS.md), [manual](../../docs/howto/open-ml-rows.md)): `test_mlrows.py` (GPU-free tests of `scripts/apps/mlrows/`: hashed locks, flavour choice, model check, judges, environment, wiring, GPU-use judge) and `mlrows_mutants.py` (two named killers per mutant, run by `ci.yml`).
