# Vast GPU CI broker

The broker connects GitHub Actions to rented NVIDIA GPU machines on Vast.ai so kayfabe can run GPU checks beyond the hardware available to hosted runners. It is a Cloudflare Worker backed by a Durable Object ledger that tracks rentals, deadlines and spending. GitHub asks for a check; the Worker handles the provider account and relays the box's result and bounded logs back to the runner.

## How a GPU check uses it

1. The [GPU workflow](../../.github/workflows/gpu-vast.yml) decides whether a master push or PR needs a check. It fetches the controller from the trusted master base, without checking out proposed code on the GitHub runner. Two independent path decisions must agree before it reports a check as not needed.
2. The controller presents GitHub OIDC identity. The Worker checks the pinned workflow, the exact run attempt's owner triggering actor and the exact tested commit, then applies offer eligibility, concurrency and spending policy before creating a rental.
3. The box bootstrap checks out that exact commit and runs the [repository-side payload and reporter](../../scripts/ci/README.md). Its scoped run token lets it report progress, logs and a structured result, request its own destruction, and obtain permitted private asset download URLs.
4. The GitHub controller independently validates the result bindings and log hashes, scrubs displayed output and supplies the workflow's `gpu-vast` status. After fetching the final log it requests immediate destruction; result grace and Worker deadlines provide fallback cleanup.

## Trust and spending boundaries

The Vast box and its operator are untrusted. The Vast account key stays in the Worker, outside GitHub secrets and rented machines. A box receives only its run capability and, when configured, a Cloudflare Access service-token pair. GitHub communicates through the broker and accepts no executable or build artifact from the box. A GPU PASS is an unattested tripwire: a dishonest box or Worker can forge a verdict. It supplies no release provenance. See the [trust and release trace](../../docs/design/V3_CI_VAST_RUNNER_TRACE_20261022.md).

The ledger reserves liability before rental and applies daily and rolling budgets, a prepaid credit reserve and limits on concurrent work. The default check policy is $1 effective/hour, $6/day, $30 per rolling 30 days and an $8 credit reserve, with one active rental and a 300-minute hard deadline for the check profile, which runs the Windows stage of the master gate ([howto](../../docs/howto/gpu-vast-windows.md)); the sweep keeps three hours. The separate owner-dispatched sweep profile has its own hourly and daily caps. Transfer allowances are reservations, not provider-enforced usage limits. Missed cron runs or provider failures can leave a box billing beyond its deadline; these controls do not guarantee a hard external bill ceiling.

## What the runner shows of the box log

STATUS: LIVE as code on branch `claude/runlog-20261105`, 2026-11-05; not yet on master, so no GitHub run has used it. Every behaviour below is CODE READ plus the GPU-free tests in `test/runner-runlog.test.js`; none is MEASURED on a real run. The box log is untrusted text: every displayed line is prefixed `[untrusted] ` and passes `sanitise()` in [runner.py](runner.py) (redaction of the Worker origin, path, Access values and OIDC token, applied on the raw text and again after the character strip so that a value split by a control character cannot reassemble; terminal sequences; characters outside printable ASCII and Latin-1 letters; `Bearer` and GitHub tokens; runs of 32 or more identifier characters; URLs; `::` and `##[`; lines over 1000 bytes). The runner writes no file and uploads nothing: `gpu-vast.yml` has no `uses:` step in the job that holds the Worker credentials. The full log is meant to come from the Worker (public logs bucket), not from the runner.

- **Display budget**: 65,000 bytes (unchanged) and 270 lines (was 200). The extra lines hold the key lines and the census below. The live head (24,000 bytes / 100 lines) is shown while the run polls; the final group then shows key lines, the middle-drop marker with its census, and the tail (16,000 bytes / 70 lines).
- **KEY LINES**: up to 60 lines and 12,000 bytes, each cut at 400 bytes, picked from the whole sanitised log by the fixed table `KEY_PATTERNS` in runner.py, never by the box: stage start/end, `KF_FINAL_SUMMARY`, `stage name=`, `FAST_CELL_ARM`/`BARE_CELL_ARM`, `WINDOWS_GATE_PASS|FAIL|RETRY|TIMING`, xid-watch before/after/delta and `NEW HOST Xid`, `GATE_HOSTREF`, and any `FAIL`/`FAILED`/`REFUSED`. Non-PASS lines come first, then the first and last line of each category, then the rest in log order. Each line is cut to 1000 characters before any pattern runs, so the work per line is bounded. A box line that imitates a key line is displayed like any other: `[untrusted]`. The verdict is the Worker's validated status, not any line. Each key line is a copy and is tagged `[key] ` (`[untrusted] [key] [box] ...`), so it never reads as the box line it copies: `gate_reuse.py` `summary_lines`, the only reader of the job log (a test asserts that), requires exactly one `[untrusted] [box] KF_FINAL_SUMMARY_BEGIN` line, and a regression test feeds it the runner's complete PASS output (key lines, census, tail). Shown only when the head did not already show the whole log.
- **Census of the dropped middle**: next to `[N bytes dropped from the middle]`, up to 12 lines `[dropped N lines / B bytes: <prefix>]`, largest bytes first, then one `[dropped others: N lines / B bytes]`. The prefix is the line's first tokens (after `[box] ` and a dmesg timestamp) up to and including the first token holding a number or `=`, numbers and hex shown as `#`, at most 60 characters, sanitised and displayed `[untrusted]`. Beyond 12 groups, and beyond 5,000 distinct prefixes, lines are only counted. Bytes are sanitised bytes; the middle marker keeps the raw count.
- **Honest markers**: `[N bytes removed by sanitisation]` (redaction, control sequences, characters outside the allow-list, lines cut at 1000 bytes) and `[N bytes dropped by size cap]` (whole lines that the line or byte cap did not show) are separate; the closing `[N bytes dropped by runner display budget]` counts only the second kind. The old single marker `[N bytes dropped by runner]` no longer exists.
- **DIAGNOSTICS**: when a run was admitted but ended without a PASS or FAIL result that passed the runner's checks, the runner prints, before the final group, the Worker's last polled state, the Worker-clock timestamps it returned (`started_at`, `last_post_at`, `result_at`, `ended_at`), the runner's own clock since the rent request, the last log text and the newest `KF_PAYLOAD_STAGE_START` line (name cut to `[A-Za-z0-9_.-]{1,40}`, displayed `[untrusted]`), and the Worker states its polling saw. It states no cause. "The last lines ... are shown below" is printed only when the Worker returned log text. It uses only fields the protocol already returns. The Worker does not return the provider's instance status, the reason it closed a run (for example `RUN_GONE`) or per-state entry times; those need a Worker-side addition.

## Current setup and further reading

[CODE READ: wrangler.toml, gpu-vast.yml, gpu-vast-sweep.yml] The public broker origin is
`https://ci.kayfabe.dev`; Worker var `WORKER_URL` and GitHub environment `gpu-vast` variable
`VAST_BROKER_URL` must match because the origin is the OIDC audience. The path and both Access
credentials stay secrets. Protect the whole `ci.kayfabe.dev` hostname with Access and the same
runner/box service tokens; keep workers.dev as a fallback only if it also stays behind Access.
The admin UI uses `https://admin.kayfabe.dev` and its separate owner-identity Access app. Before
the first var deploy the coordinator deletes the old `WORKER_URL` secret, following [DEPLOY](DEPLOY.md).

Renting is enabled by the deployed vars file, which is kept outside the repository ([deployment guide](DEPLOY.md)); `RENTING_ENABLED=false` in the committed [Worker configuration](wrangler.toml) is only the safe default. `RUN_TOKEN_SECRET` signs run capabilities and `OWNER_STOP_TOKEN` controls emergency cleanup. As of 2026-10-25 acceptance of the current hosted workflow with the protocol-7 payload is still open; historical local smoke results do not establish it.

- [Threat model](docs/THREAT_MODEL.md): who is trusted, and which behaviours are intended design rather than vulnerabilities.
- [Box protocol](docs/BOX_PROTOCOL.md): bootstrap environment, callbacks and result contract.
- [Placement decision](docs/PLACEMENT.md): US/Canada offer preference and the proposed ledger location.
- [Evidence summary](../../traces/vast_broker_summary/README.md): recorded runs, retained evidence and its limitations.
- [Internal implementation and evidence notes](docs/IMPLEMENTATION_NOTES.md): the former README's dated rounds, corrections and superseded behavior.

After deploying the matching Worker bundle, use a fresh GPU-relevant commit to start validation; reruns retain the original trusted BASE pin.

CODE READ (`src/provider-fault.js`, `src/ledger.js`, `runner.py`): provider failures stop and destroy the box before one replacement on a different known host. Two failed machines condemn their shared host; static seed `224600` comes from runs 40425285335 and 40478005010. An exhausted replacement stays a non-passing infrastructure error, with no kayfabe verdict. See [provider-fault handling](../../docs/howto/vast-provider-faults.md).

CODE READ 2026-10-27 (`src/github.js`): attempt3 and later have no special authorization ceiling. Admission reads the signed attempt's REST resource and retains strict actor, repository, status and SHA bindings. For a 401, use the fixed field-only observability code; see [rerun authorization](../../docs/howto/vast-rerun-auth.md).

CODE READ 2026-10-27 (`gate_reuse.py`): proposed master-push reuse preserves the existing GPU job with readable original result logs and summary. It requires identical trees and complete GitHub status/CI/run evidence; uncertainty uses the existing gate. See [reuse behavior](../../docs/howto/gpu-gate-reuse.md).
