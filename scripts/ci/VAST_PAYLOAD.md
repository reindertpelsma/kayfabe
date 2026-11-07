# Thin Vast payload

STATUS: LIVE, 2026-11-07 (the apps stage is a DRAFT, disabled by default, no box has run it under the payload; host claims added by branch `claude/setup-picker-20261106`; release-tier
extras added by branch `claude/worker-tier-20261105`; the evidence bundle added by branch
`claude/ci-evidence-bundle-20261106`; the run report added by branch `claude/report-20261107`; none has
run on a box).

[CODE READ: vast_payload.sh, vast_payload.py] The generic onstart invokes
`bash scripts/ci/vast_payload.sh <sha> <nonce>` from the exact approved checkout.
The payload uses the existing disposable-box flows, rather than another provisioning path:

1. provision_full.sh: fresh box, current host driver, bench tree and this revision's kf3.
2. cuda_ladder.sh host: cup2/cup3/cup8/cup8bench, one repetition, before guest workloads.
3. merge_check.sh: crate census, gates, rebuild, raw and thin suites, birth census. [2026-10-26]
   It no longer runs the GPU-free `cargo test` of every kf-* crate (CI's `stable` job does); the
   /dev/kvm-gated tests are an opt-in stage, `KF_MERGE_KVM_TESTS=1` (kvm_tests.sh).
4. cuda_ladder.sh guest: the existing fat guest and unchanged ladder hooks, four rungs.

[CODE READ: vast_payload.py] Helpers that fetch origin see only this approved local SHA.
The adapter validates exact revision and the full RC/READY/EXIT/PASS evidence, streams stage
logs, writes per-stage duration lines, and atomically preserves finished rows in
/workspace/kf-result.json. The PR #104 reporter supplies the run identity and heartbeats.
A first red cancels later work. Soft-deadline expiry records FAIL/timeout with finished rows.
No run/account credential is required by the payload; the reporter owns the run capability.

[CODE READ: vast_payload_hooks.sh, vast_payload.py xid_ok] The existing host_xid_watch is
used both per raw/guest arm and per full stage. One raw victim_release missing-page fault at
0xa11000000 is the only first-version admission, with exact PDE/write type and a CE client.
All other new records refuse. S4's exact-channel guard has one replacement point, xid_ok;
the raw marker currently carries the VA, not its channel. No per-die allowance is introduced.

[CODE READ: test_vast_payload.py, test_vast_payload_v7.py, payload_mutants.py] GPU-free checks:
`python3 -m unittest discover -s scripts/ci -p 'test_vast_payload*.py'`
and `python3 scripts/ci/payload_mutants.py`. Black uses line length 100; shellcheck covers both
shell entrypoints. [Evidence](../../traces/vast_payload_v7_20261018/README.md).

[CODE READ: vast_report.py load_provider_env] The reporter's check-env step supports Vast VMs
that persist contract values in /etc/environment instead of exporting them to a non-login
startup process. Only missing protocol names are parsed as data; process values win and
unrelated provider/account credentials are never imported. The daemon still uses the validated
root-only env file. [Recovery evidence](../../traces/vast_payload_v7_20261018/PROVIDER_ENV.md).

[CODE READ: vast_payload.py provision_grade] Cargo outputs stay in the approved checkout's
`target/`, where the existing bench and fast-guest helpers resolve their clients. Hardware is
queried again before grading provisioning, including its failure path. The live run exposed
both integration defects; their fixes have red/green and mutation coverage but no fresh VM run.
[Final live evidence](../../traces/vast_payload_v7_20261018/LIVE_RUN.md).

[MEASURED: 2026-10-26, run 40241597724, PR 130 head c9d33b724] The live gate ran end to end on
an RTX 3060 (setup cache restored in 17 s, `BOX_RC=0`) and ended GRACE/FAIL, but the job log held
only the first ~25 KB of box output: the unredirected `cargo build -q` of provision_full.sh
prints every rustc warning (`-q` and CARGO_TERM_QUIET do not silence them), and the runner's
display budget (65000 bytes, 200 lines) was spent on the first lines.
[CODE READ: vast_payload.py final_summary, runner.py show_head/show_tail, vast_report.py
_fit_tail] Four changes make the verdict visible: the payload ends with one bounded
`KF_FINAL_SUMMARY_BEGIN .. END` block (stage rc/duration, suite rows, first failing line, the
provisioning step log tail; <= 4 KiB); the runner shows the head (24000 bytes/100 lines) and the
tail (16000 bytes/70 lines) of the box log with `[N bytes dropped from the middle]`; the box
reporter drops the oldest unsent text, never the newest, and reserves posts, bytes and requests
for the end of the log; the ladder build writes warnings to /root/prov/ladder.log and prints
errors. GPU-free checks: test_vast_payload_summary.py, test_vast_quiet_cargo.py, the Reporter
tests of test_vast_box.py, ci/vast-broker/test/runner-head-tail.test.js, and the mutation
harnesses payload_mutants.py, vast_box_mutants.py, visibility_mutants.py (two killers each).
The runner is the PR's base runner on its own check run, so the display change takes effect
only after merge.

[MEASURED: 2026-10-26, run 40263112941, PR 143 head, RTX 3060] provision PASS (838 s), cuda_host
PASS (17 s), merge_bar FAIL rc=101 after 1855 s with `first_fail ... line=[redacted] --- FAILED`,
cuda_guest NOT_RUN. [CODE READ: merge_check.sh, vast_payload.py merge_rows] The stage ran
`cargo test -q --no-fail-fast -p kf-* ...` over all 22 kf-* crates (44 `-p` words) on the box, as root,
with /dev/kvm present and without KAYFABE_NO_KVM; `set -e` ended the script on cargo's rc=101, so no
`TESTS passed` line existed and merge_rows reported `kf_tests`. Which test failed is not recoverable
from that run. [CODE READ: ci/vast-broker/src/sanitize.js, runner.py scrub; rustc 1.98.1 output]
libtest's terse mode prints `<test name> --- FAILED`, and both scrubbers blank any run of 32 or more
`[A-Za-z0-9_-]` characters, so a long snake_case test name became `[redacted]`.
[CODE READ: grep of crates/kf-*] No kf-* test opens /dev/nvidia*, a kf3 binary or QEMU; the only
box-specific tests are the /dev/kvm-gated ones in kf-linux-raw and kf-chan, which CI skips
(KAYFABE_NO_KVM=1) and counts as reached. The box bar therefore keeps no crate test by default.
Decisions, each with GPU-free tests and two killers per mutant (test_vast_merge_bar.py,
test_vast_payload_failures.py, ci/vast-broker/test/log-identifiers.test.js and
runner-identifiers.test.js; mutants: merge_bar_mutants.py, payload_mutants.py):
- merge_bar drops the all-crates `cargo test` and its `kf_tests` row; the crate census stays as a
  sparse-checkout guard. `KF_MERGE_KVM_TESTS=1` runs `cargo test -p kf-linux-raw -p kf-chan` and is
  graded only when requested: `KVM_TESTS passed N failed 0 kvm_ran>0 kvm_skipped=0`, else `kvm_tests`.
- A failed stage's FINAL SUMMARY now carries `failed_test stage= name=` (at most 20, then
  `failed_tests_more`) and `panic stage= line=` (first panic and its message line), read from the
  stage's logs. Test-name-shaped runs of 32+ characters are folded into pieces of at most 30 joined
  by `~`, so a Worker that has not been redeployed still shows them. Nothing else is folded.
- Both scrubbers keep a run of 32+ characters only if it is a snake_case test name: lowercase words
  joined by `_`, at least three, each at most 20 characters, at most 128 characters, at most one
  digit in five. Hex, base64url, mixed case, hyphenated and digit-heavy strings, token families and
  configured secrets are still redacted (ci/vast-broker/test/fixtures/scrub-vectors.json pins the
  three copies of the grammar). sanitize.js runs in the deployed Worker: it needs a deploy; runner.py
  is the base runner and takes effect after merge.

## Host claims for the setup picker (2026-11-06)

CODE READ (`vast_payload.py` `host_claims`, `cpu_claims`, `refresh_claims`; `vast_report.py`
`clean_claims`, `final_result`); GPU-free tests only (`test_vast_payload_claims.py`, nine new cases in
`payload_mutants.py`); no box ran this. The payload records, as `host_claims` in
`/workspace/kf-result.json`: the CPU vendor (`vendor_id` of `/proc/cpuinfo`: GenuineIntel is `intel`,
AuthenticAMD is `amd`, an arm64 machine is `arm`, anything else `other`), the sanitised CPU model name
(at most 64 characters), the architecture, the guest OS versions of the harness images
(`ubuntu-24.04`, `11-ltsc-2024-26100`; a test pins them to the scripts that install those guests),
the Linux guest driver from the receipt `guest.qcow2.STAGED` that `provision_bench_tree.sh` writes,
and, only when the Windows stage is part of the run, the Windows guest OS version and driver `580.88`.
The Windows values are the harness's configured constants, sent even if the stage fails before the driver
is installed: they say what the harness installs, not what it read from the guest. The reporter
forwards the block only when the Worker set `KF_HOST_CLAIMS=1`; the fields are checked again by the
Worker. They are claims, not evidence, and never change a verdict. No serial, UUID, address, hostname
or machine id is read: a value shaped like one is dropped. See
`ci/vast-broker/docs/BOX_PROTOCOL.md`, "Optional host claims".

## Release-tier extras (wtier, 2026-11-05)

CODE READ (`vast_payload.py` `tier_requested`, `specviewperf_rows`, `vast_stage_specviewperf.sh`,
`vast_report.py` SPEC); GPU-free tests only (`test_vast_payload_v7.py` ReleaseTierPayloadTests,
`test_vast_payload_tiers.py`, three new cases in `payload_mutants.py`); no box ran this. The Worker
may send three optional variables, each validated by the reporter and popped before any stage:
`KF_TIER` (small, middle, large, adhoc; informational), `KF_WINDOWS_RUNS=2` (release only: a second
`windows_gate.sh` run on the same box, row `windows_gate_2`, when no second machine offers the die;
refused without the Windows stage; it needs 45 minutes left, an INFERRED margin over the second
runs of 906 s and 1795 s on 2026-11-04 at rev `0c515b3b`) and `KF_STAGES` (release only,
colon-separated slot names). Slot `specviewperf` runs `vast_stage_specviewperf.sh` last: it calls
`scripts/bench/box/specviewperf_stage.sh` when that lane exists, keeps the lane's whole output in a
0600 file under `/workspace/kf-ci-private/` (never streamed), and prints only the verdict line; without
the lane the row is `specviewperf_retry_not_built`, never a pass. Slots `two_guests` and `trio`
are reserved and DESIGN-ONLY: requesting them is a preflight failure. A malformed value, a stage
outside the large tier or a duplicate is refused before the first stage.

## Evidence bundle (2026-11-06)

CODE READ (`vast_evidence.py`, `vast_onstart.sh`, `ci/vast-broker/src/evidence.js`); GPU-free tests
only (`test_vast_evidence.py`, `evidence_mutants.py`, the Onstart cases of `test_vast_box.py`); no box
ran this. After the payload ends, whatever its exit status or timeout, and before the result is
reported, `vast_onstart.sh` runs `python3 -I vast_evidence.py run` (skipped when the checkout has no
such file). It is bounded by `min(110 s, KF_SOFT_DEADLINE + 150 s - now)` and by `timeout`; it always
exits 0, prints its outcome into the log and writes neither the result file nor the log. It lives in
onstart, not in the payload, so a payload killed at the soft deadline still gets its evidence, and the
payload still gets no run capability.

Bundle `.tgz`, at most 8 MiB compressed and 48 MiB unpacked, `MANIFEST.json` first. Sources and keep
order (tier 0 is kept longest; when a cap binds the LAST tier goes first, and within a tier the oldest
item first):

| Tier | Holds | When the cap binds |
|---|---|---|
| 0 | `MANIFEST.json`; `host/` state: `free -m`, `df -h`, `uname -a`, `date`, `uptime`, `ps` lines of qemu and swtpm, the first 60 lines of `nvidia-smi -q`; `/workspace/kf-result.json` | never dropped |
| 1 | `log/output.log` (the last 3 MiB of the payload log); the gate and provision verdict logs `/root/prov/*_windows.log` and `prov.log` | never dropped; the larger one is halved (newest bytes kept) until the bundle fits, and the manifest records the cut |
| 2 | `host/dmesg_tail.txt` (the last 256 KiB of `dmesg -T`); the payload's own `/workspace/kf-ci-evidence/*` (stage `*.run`, diagnostics); the newest `timeout_*.png` screendump of the Windows VM; other `/root/prov/*.log` | dropped after tiers 5, 4, 3 |
| 3 | the gate evidence tgz `/root/prov/*_windows.tgz` (expanded member by member, each cut to its last 2 MiB; a directory is read only when no tgz of the same name exists) and other `/root/prov/<dir>/` text; the Windows VM `logs/*.log|txt|jsonl|err|out` (newest 40, last 512 KiB each); the 3 newest screenshots | dropped after tiers 5 and 4 |
| 4 | `/workspace/bench/*.out` and `xid_*.log` (newest 30, last 256 KiB each) | dropped after tier 5 |
| 5 | screenshots 4 to 6 (at most 6 PNGs in all, each at most 2 MiB); gzip'd VM logs | dropped first |

Never collected: `secrets/`, `tpm/`, `firmware/`, `disk/` and `.ssh` directories; ssh keys and other
names matching the forbidden-name list; disk images; anything outside the roots above. No symlink is
followed at any level, roots included: every path is checked with `lstat` and every file is opened
with `O_NOFOLLOW | O_NONBLOCK` and checked to be a regular file after the open (so a FIFO cannot block
the helper). A directory swapped for a symlink between the check and the open of a file below it is
a race that remains. Names inside the bundle, and the skipped names in the manifest, are reduced to
`[A-Za-z0-9._/-]`.

Order of work (review of PR #274, finding F2): every FILE is collected first and the host commands
(`free`, `df`, `uname`, `date`, `uptime`, `ps`, `nvidia-smi -q`, `dmesg -T`) last, so a hung GPU or a
stale mount costs only the commands. Each command runs in its own process group with its output in a
temporary file, gets 6 s, is then killed with SIGKILL (the whole group) and waited for at most 2 s more,
and a process stuck in the kernel is abandoned. All commands together get 25 s; a command that does not
fit is replaced by a marker file `[kf-evidence: skipped, out of time]` and listed in the manifest.

Scrubbed in text members, counted in the manifest: GPU and board UUIDs (a distinct synthetic
`fa4efa4e-...` per value, as `trio_stage.sh` does), serial numbers (`Serial Number :` lines and
`serial=` values), MAC addresses (QEMU's own `52:54:00:*` kept), public IPv4 and IPv6 addresses
(private, loopback and link-local kept; a dotted four-number version string such as `5.15.0.91` is
also replaced, an accepted loss). Not scrubbed: PNG pixels. The repository's display scrubbers
(`sanitise()`, `sanitize.js`) blank every run of 32 characters and would destroy hashes and
addresses, so they are not used here: the bundle is private, not displayed.

Scanned on the RAW bytes of every member before it is packed (a hit refuses the whole upload, prints
`KF_EVIDENCE_REFUSED reason=scan_hit class=<class> member=<name>`, never a value; when the hit is in
a file NAME the member is printed as `name-<10 hex of its SHA-256>`, so a secret-shaped name never
reaches the log): the run token, the
broker URL and its secret path, the Access pair, the runner nonce (all from the box environment), the
guest password and the harness ssh key (every file under `$WINVM_ROOT/*/secrets/`, the files
`windows_gate.sh` scans for), `~/.ssh/id_*`, a Vast key file or variable, JWT-shaped strings, `Bearer
<20+ characters>`, presigned-URL parameters, private-key blocks, GitHub tokens, `CF-Access-Client-*`
headers and URLs with credentials. Values shorter than 6 characters are not scanned. Cuts (review of
PR #274, finding F3): each member is scanned at its full length with the 8 KiB that precede the size
cut (a log tail, a gz tail, a tgz member cut to its last 2 MiB) before it is cut, the tier 0/1 logs
are scanned before the size-halving cut, `MANIFEST.json` is scanned before it is packed, and the
final bytes are checked once more for exact values; a 16-character prefix or suffix of the run token,
the Access secret, the runner nonce, the guest password, the ssh key lines, a Vast key or the secret
broker path is a hit too (`<class>_fragment`: a file that starts or ends in the middle of a secret).
Every pattern is anchored on a literal and bounded; the JWT test is a linear scan, not a regex with a
nested quantifier (F4: the reviewer timed the first version at 1.30 s for 64 KiB of `eyJ-`, rev
f345f6aad, 2026-11-06, scratch harness, Python 3.14.4, growing with the square; `test_scanning_and_scrubbing_are_linear_on_hostile_input` now feeds 1 MiB
of each hostile shape). Not covered: a
secret that is encoded (base64, URL-encoded) or split across lines, and anything inside a PNG.
A hostile guest can plant a secret-shaped string in a file it controls to make the scan refuse the
upload: the evidence is best effort, the refusal line still names the class and member, and nothing is
uploaded in that case.

Upload: `POST <KF_BROKER_URL>/evidence/<run>-<attempt>` ([box protocol](../../ci/vast-broker/docs/BOX_PROTOCOL.md),
"Evidence upload"), HTTPS only, no redirect, no proxy, the token and the Access pair only as headers,
at most 3 tries on a network error, `409 BUSY`, `409 EVIDENCE_PENDING` (an earlier try still being written), 429 or 5xx. Only the Worker's `409 EVIDENCE_EXISTS`
counts as stored. The log line is `evidence uploaded: <run>/<attempt>
<bytes> sha256 <first 12 hex>` (with ` (already_stored)` after an `EVIDENCE_EXISTS`), `KF_EVIDENCE_REFUSED reason=...`
(scan hit or too large), or `KF_EVIDENCE_SKIPPED reason=...` (`upload_http_<status>`, `time`,
`environment_contract`, `build_error:<Class>`, `helper_exit_<rc>`). The runner is not changed: the line
is at the end of the log, in the tail the runner shows, and a refusal matches its existing `REFUSED` key
line. Retention (30 days) and reading are in [DEPLOY](../../ci/vast-broker/DEPLOY.md), "Evidence bucket".

GPU-free checks: `python3 -m unittest discover -s scripts/ci -p 'test_vast*.py'` and `python3
scripts/ci/evidence_mutants.py` (every mutant must fail two tests).

## Run report (2026-11-07)

[CODE READ: vast_run_report.py, vast_onstart.sh; GPU-free tests only, no box has run it] After the payload has
ended and before the result is reported, `vast_onstart.sh` runs `vast_run_report.py` (only when the Worker set
`KF_RUN_REPORT=1`; at most 30 s, never past the soft deadline plus 150 s; exit status ignored; one outcome line in
the log: `KF_RUN_REPORT_SENT bytes=<n> subruns=<n>` or `KF_RUN_REPORT_SKIPPED reason=<word>`). It builds one
`schema 1` document from output the payload ALREADY produces and posts it to `POST <KF_BROKER_URL>/run-report/<run>`
([box protocol](../../ci/vast-broker/docs/BOX_PROTOCOL.md), "Optional run report"). The result file is never touched,
and nothing is invented: a field without a source stays empty.

| Report field | Source (already printed or written by the payload) |
|---|---|
| `suites` | the graded rows of `/workspace/kf-result.json` (the same rows the `suite name=X pass= fail= skip=` summary lines print) |
| `stages` | the final summary's `stage name=X rc= duration_ms= result=` lines (a stage with only a `KF_PAYLOAD_STAGE_END` line is `UNATTRIBUTED`; `TIMEOUT` is FAIL, `PROVIDER_FAULT` is RETRY) |
| `tests`, host | `BARE_CELL_ARM arm= verdict= ms=` (suite `raw_suite`) and `CL_ROW mode=host rung= wall_ms=` (suite `cuda_host`) |
| `tests`, Linux guest | `FAST_CELL_ARM arm= verdict= client_ms=` (suite `thin_suite`, else `secs`) and `CL_ROW mode=guest rung= CUP8_WALL_S=/BENCH_measure_WALL_S=` (suite `cuda_fat`) |
| `tests`, Windows | `GATE_PROBE_RESULT <name> ... rc= secs= timeout=` (suite `windows_gate`; a repeated probe is `<name>#2`) |
| `perf` | the `CL_BENCH` lines of `cuda_ladder.sh` (the lines `scripts/bench/perf_summarize.py` reads): `BSUM N= ... gflops= batch_gflops= med_ms= h2d_ms= d2h_med_ms= ...`, `BENCH_INIT_MS/CTX_MS/MODULE_MS`, and for N of 1024 or more the derived `h2d_GBps` / `d2h_GBps` by the formula of `perf_summarize.py`; a guest row carries the host row of the same name as `ref_value` and their ratio as `ref_ratio` (a same-box reference the box recorded itself) |
| `windows` | `WINDOWS_GATE_PASS/FAIL/RETRY` (`probes=`, `resets=`, `known_live_dumps_193=`) and `WINDOWS_GATE_TIMING ... install=<n>s` |
| `guest_drivers`, `subruns[].host_driver` / `guest_driver` | `hardware.driver_version` (the box's `nvidia-smi`), and the host claims of the result file: the Linux guest driver from the `guest.qcow2.STAGED` receipt (as `guest_driver_receipt` does), the Windows driver constant of the harness |
| `apps`, and `tests` of suite `apps_linux` | the `APP_ROW name= os=linux result= secs= note=` lines of the apps stage (`scripts/bench/box/apps_gate.sh`; the stage is disabled by default, so the list is empty until the owner enables it). The suite rows `apps` and `apps_retry_<class>` belong to the Linux subrun (a failed apps stage fails it) |

Subruns of a host session (one rental): `host` (suites `cuda_host`, `kvm_tests`, `gates`, `kf3_build`,
`raw_suite`), `linux-<guest driver>` (`thin_suite`, `birth_census`, `cuda_fat`) and `windows-<driver>`
(`windows_gate`, `windows_gate_2`; `windows` with no driver when none is known). `provision` belongs to the session, not
to a subrun. A subrun with no graded row is `NOT_RUN` with a reason (`Windows stage not requested for this
profile`, or `not reached: <failing suite>`); a partial one is `UNATTRIBUTED`; a Windows gate that printed
`WINDOWS_GATE_RETRY` is `RETRY`. A future small tier adds more `windows-<driver>` subruns to the same session.
`duration_ms` of a subrun is the sum of the timed tests in it. Its position in the published log is found
by the Worker from the stage markers (`KF_PAYLOAD_STAGE_START/END name=`), not sent by the box.

Bounds on the box (the Worker repeats and enforces them): at most 256 KiB of JSON (the largest arrays are halved
until it fits), 2000 tests, 200 perf rows, 64 stages, 20 suites, lines of at most 8192 characters parsed, a log of
at most 32 MiB read. SPECviewperf lines never become a row (the stage prints only its verdict word).

GPU-free checks: `python3 -m unittest scripts/ci/test_vast_run_report.py` (from `scripts/ci`) and the run-report cases of
`test_vast_box.py` (`Onstart`); mutants: `python3 scripts/ci/run_report_mutants.py` (every mutant must fail two tests).

## Apps stage (DRAFT, DISABLED BY DEFAULT, 2026-11-07, lane appslinux)

[CODE READ: vast_payload.py `APPS_STAGE_ENABLED`, `apps_requested`, `apps_rows`; scripts/bench/box/apps_gate.sh]
The owner's ruling (2026-11-07) is that the release gate includes the Linux CUDA/graphics/AI apps of
`scripts/apps/` in the guest. The stage `apps` runs `apps_gate.sh` after `cuda_guest` and before
`windows_gate`, over the gate list `scripts/apps/gate_list.txt` (the apps that passed on bare metal AND in the
guest on the measured box, [traces/apps_linux_20261107](../../traces/apps_linux_20261107/README.md)). It is off
until the owner sets `APPS_STAGE_ENABLED = True` in `vast_payload.py`; then the tier decides: `middle` and
`large` run it, `small` and `adhoc` never do. `KF_STAGES=apps` stays refused (the tier decides, not a Worker
field), so no Worker change is needed: the Worker bundle (`ci/vast-broker/src` plus its config, hash in
`ci/vast-broker/deployment.json`) is not touched, and the box scripts reach the box through the git checkout of
the tested SHA. Grading: one `APPS_STAGE_PASS`, exit 0, exactly one PASS `APP_ROW` for every gate-list name and
no other, else the row `apps` (a gate app failed in the guest, a kayfabe finding) or `apps_retry_<class>`
(`not_built`, `time`, `box`, `operator`, ...: not a verdict on the guest; the Worker already reads
`<x>_retry_<y>` rows as RETRY). The stage has its own time budget (`KF_APPS_BUDGET_S`, default 3000 s) and does
not start with less than `APPS_MIN_REMAINING_MS` left. Per-app rows leave the box as `APP_ROW` log lines and
become `apps[]` and `tests[]` rows (suite `apps_linux`) in `report.json` through `vast_run_report.py` (PR #282).
GPU-free checks: `test_vast_payload_apps.py`, `test_apps_matrix_table.py`, the `apps-*` cases of
`payload_mutants.py` (two killers each).


## Experimental nested feasibility stage (J3, disabled by default)

[CODE READ; hardware NOT MEASURED] `KF_NESTED=1` requests the optional `nested` stage only for
`KF_TIER=large`. Absent/0 leaves it disabled; small/middle/adhoc/unknown reject the opt-in at
preflight, and `KF_STAGES` cannot select it. The stage needs an already running, exclusively
assigned outer Linux guest over SSH, prebuilt revision-specific kf3 QEMU and fastguest artifacts,
and at least 45 minutes before the payload's soft deadline (an INFERRED budget). It runs one
raw CE selection inside one inner thin guest, with KVM or TCG, after Linux CUDA/apps and before
Windows. PASS requires complete checked output and clean exits; missing artifacts, SSH faults
and exhausted budgets become failing `nested_retry_<class>` rows. Run-report tests of suite
`nested` and its suite rows belong to the Linux subrun. No Worker deployment or automatic
outer-guest boot is included. The preparation contract, unmeasured assumptions and coordinator
hardware checklist are in [the nested design](../../docs/design/V3_NESTED_KAYFABE.md).
