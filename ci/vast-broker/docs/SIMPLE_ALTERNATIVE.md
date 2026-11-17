STATUS: DESIGN-ONLY 2026-10-25. Prototype code and GPU-free tests exist in `scripts/ci/vast_simple/`;
no real Vast rental has run it yet. The Worker stays LIVE until the owner decides.

# A simple alternative to the Worker: one script, one private runner

Tags: `CODE READ <date>` (read in the repository), `INFERRED` (reasoning, not run), `MEASURED
<date>` (a run, named). Source of the decision: owner, 2026-10-25.

## Why

The Worker (`ci/vast-broker`) is built for a threat model that the owner has since narrowed.
GPU-free tests run first on official GitHub CI, so a box cannot forge them. A malicious box can
only give a wrong pass/fail bit for code that is already trusted; it cannot touch binaries.
Cloudflare Access is only an anti-bot gate. The one real risk is a leaked Vast key (spend). That
is bounded by a dedicated low-balance Vast account and by keeping the key off the public repo.
Under that model most Worker machinery buys nothing.

## What it is

`scripts/ci/vast_simple/run.py --sha <tested> --profile check|sweep`, with `VAST_API_KEY` in the
environment. Stdlib only. Four modules: `run.py` (flow and SSH), `vast_api.py` (console client),
`offers.py` (offer rules), `reaper.py` (backstop). Steps:

1. List offers with the Worker's query and rules, from the same `defaults.json` (CODE READ
   2026-10-25: `src/vast.js offers()`, `src/policy.js`, `src/offers.js`). A local bad-machine file
   adds to the built-in list.
2. Create the VM as `src/vast.js create()` does: `PUT /api/v0/asks/<id>/`, `vm`, the same image,
   `ssh_direct`, label `kf-ci-simple-<epoch>-<8 hex>`. The onstart is a no-op; the account SSH key
   is pre-registered on the Vast account, so the box needs no key delivery.
3. Wait for `running`, then for SSH on the direct `public_ipaddr` and the `22/tcp` host port.
   Boot deadline 15 minutes. If the box never answers, destroy it and try the next machine, once.
4. Over SSH: `git clone --filter=blob:none`, fetch and check out the exact SHA, verify `HEAD`, run
   the payload under `timeout`, stream a capped log (4 MiB), read `/workspace/kf-result.json`
   back with `head -c`. Hard deadline from `defaults.json` (180 min), soft margin 10 min.
5. Always, in a `finally`: destroy by instance id (only if the instance's own label starts with
   `kf-ci-simple-` and equals the label this run created), then prove absence through
   `GET /api/v1/instances`. A box that cannot be proven absent is a LEAK.
6. Exit code is the verdict: 0 PASS, 1 FAIL, 2 UNAVAILABLE (no verdict about the code), 3 LEAK.

PASS needs payload exit 0, `verdict == "PASS"`, no failures and no skips, at least one pass,
known hardware with compute capability at least 750 (the same bar `vast_report.py` applies).

`reaper.py` lists instances (instances-v1, paginated) and destroys `kf-ci-simple-*` older than
hard + 30 minutes. Age comes from the epoch in the label. A malformed list (wrong shape, bad id,
duplicate, `success` not true, looping page token) deletes nothing and exits 2. A
`kf-ci-simple-` label it cannot age is reported and left, exit 3. Other labels are never read
as candidates, so Worker boxes (`kf-ci-...`) and manual boxes are out of reach.

`private-workflow.yml.template` is the matching workflow for the private repository.

## Where the key lives, and one rule that matters

`run.py` runs with the Vast key. Code from the tested SHA must therefore never run there: the
template checks out a pinned, reviewed `RUNNER_PIN` commit for the runner code and treats the
tested SHA as a string only (validated, ancestor of master, GPU-free job green). The box clones
the tested SHA; the private runner never does. INFERRED: running `run.py` from the tested SHA
would let any pull request exfiltrate the key.

## Compared with the Worker

| | Worker | Simple |
|---|---|---|
| Dropped | OIDC trust of the workflow, Access service-token pairs, Durable Object ledger, bundle hash ceremony, box-to-Worker callbacks and run tokens, `kf-box-blob-url` | none of these exist |
| Kept | offer filters and thresholds, create body, boot deadline handling, bounded log, hard deadline, destroy-and-verify, label scoping, the box payload | same rules, same data file |
| Lost | independent spend ledger (daily and rolling caps, reserve), kill switch, teardown when the runner dies | see below |

Lost, and what covers it:

- Spend ledger. Covered by the Vast-side balance of a dedicated account, plus the per-run price
  cap in the offer rules (INFERRED arithmetic from `defaults.json`: effective price at most
  1 USD/h for check, 2.5 for sweep, so at most about 3.0 and 7.5 USD per run over the 181-minute
  window). No daily cap exists in the script; the GitHub concurrency group allows one run at a
  time.
- Kill switch. Covered by disabling the private workflow, revoking the Vast key, or an empty
  balance.
- Runner dies. GitHub cancellation sends SIGINT then SIGTERM; `run.py` turns both into a
  `finally` teardown, but the platform may kill it before teardown completes (INFERRED, the
  grace period is short). The 30-minute reaper cron then removes the instance once it is
  hard + 30 minutes old (at most about 4 hours of rent, INFERRED). If GitHub itself is down, only
  the Vast-side balance applies.
- A box that lies. Accepted by the owner's model: it can flip one bit for trusted code.

## Threat model in short

Trusted: the private repo, its secrets, the pinned runner commit, the public GPU-free CI result.
Untrusted: the box (host, guest, its output; log text is data, redacted of the key before it is
printed), other Vast tenants, the tested code's behaviour on the box. Attacker goals and the
answer: steal the key (not on the box, not in argv, not in logs, not in the artifact; the
client refuses any base URL other than the Vast console or loopback; SSH has no agent
forwarding); spend (price cap, one box at a time, reaper, dedicated balance); delete foreign
instances (label prefix plus exact label, checked twice, and only by id).

## Tests

MEASURED 2026-10-25, local, no Vast: `python3 -m unittest scripts/ci/test_vast_simple.py` runs
against a fake Vast API (loopback `http.server`) and a fake SSH transport on a virtual clock.
It covers the happy path, boot never answering, a provider stuck loading, a create error that
made an instance, payload failure, timeout and transport exceptions (all destroyed), lag and
leak in absence checks, label safety, reaper fail-closed shapes, log bounds and key redaction.
`python3 scripts/ci/vast_simple_mutants.py` applies 20 mutants in 8 behaviours, each killed by
two named tests.

## Migration path

1. Merge the prototype; the Worker keeps running.
2. The coordinator runs one real check from a dedicated low-balance account (create is the part
   no test covers) and records it here.
3. Create the private repo from the template; run both paths on the same SHAs for a few weeks.
4. If results agree and the owner agrees, retire the Worker deployment and its secrets; keep
   `defaults.json` and the box scripts.

## Open questions

- The instances-v1 page parameter name `next_token` is taken from the Worker's reading of the
  response, not from the API documentation (INFERRED).
- Whether a VM created with a no-op onstart gets the account SSH key and accepts `root`
  (INFERRED from `scripts/bench/box/README.md` practice; `--ssh-user` exists for this).
- `vast_payload.sh` cannot run without the broker: it calls `vast_setup.py restore`, which needs
  the run token and `kf-box-blob-url`. The runner therefore starts `vast_payload.py` directly,
  with online `apt` and `cargo` and no setup cache. Cost in minutes is unknown; a cache path
  that does not need the Worker (public release assets, or a token carried by the private repo)
  is a decision for the owner.
- Offer-rule drift: `offers.py` is a port. A parity test that feeds one offer fixture to both
  `policy.js` and `offers.py` in CI would pin it. Not written.
- The sweep has no ledgered die rotation; `--dies` or the day index chooses the start.
- Concurrency is one box per run; the Worker allowed up to four.
- Local `black` is 26.5.1; CI pins 25.1.0 for `ci/vast-broker`. The new Python is formatted with
  the repository config and is not covered by that CI step.
