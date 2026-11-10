STATUS: LIVE, 2026-11-09. CODE READ of the implementation and GPU-free mock tests; no deployment or hardware qualification.

# Provider recovery before the first report

Owner input, 2026-11-09: batch gate #318, second run 42542135212,
reported a TU116 box stuck in `created` with the provider message
"GPU error, unable to start instance" until the boot limit. This is supplied
incident context, not a new live-provider observation by this change.

CODE READ (`src/provider-policy.js`, `src/index.js`): during BOOTING, the Worker
matches literal, case-insensitive signatures from `defaults.json.providerPolicy`.
The initial fatal rules are the GPU start error and the owned instance's exact
KVM Domain-not-found signature. The latter requires both the QEMU error marker
and the matching instance id in newly observed output. Image pull progress, checksum verification, GPU
preparation, daemon retries and other unmatched error text do not qualify.
CODE READ: before the first box report, `created`, `loading`, `starting`,
`scheduling`, `offline`, null, `running`, `error`, `exited` and `stopped` use the
same class for fatal confirmation. Changing between those states does not reset
an unchanged current fatal status or log
witness. `running` alone is not healthy evidence. A box report, or explicit recovery
progress from the data table `recoverySignatures` (`Domain successfully started`,
`GPUs ready` initially), cancels the witness. Ordinary retry/preparation text does
not override a current fatal status. A changed status message starts a new window.
CODE READ: this grouping applies only to recognized fatal evidence; it does not
change the ordinary pending-status checks or attribute unmatched errors to hardware.
CODE READ: boot log checks use only a new suffix, tracked with a bounded hashed
cursor; retained text is never rescanned. An unchanged log plus the same pending
class and status message preserves the frozen-failure witness. New progress,
changed message, a log reset or an unreadable log cancels that log witness. Cursor
and observation state persist across Worker restarts without raw provider log text.
CODE READ: recovery advances the log baseline. If that log is unreadable, continuity
is invalidated and the next readable log establishes a baseline. Status text seen
during recovery remains historical across later pending-state flaps until it changes.
New fatal text after recovery remains actionable and must pass its own full window.
CODE READ: each new log suffix is processed in order. A later progress/healthy
line from the configured ignorable-signature table clears an earlier fatal
event in that batch; a subsequent fatal event wins again. Error-only tails still
confirm after the full window. This gives the same outcome when progress is
batched with the error or delivered in a separate poll.
CODE READ: progress followed by another fatal in a batch starts a fresh
confirmation window; it cannot reuse a pre-progress witness's elapsed time.

CODE READ: confirmation requires at least two observations separated by
`observationSpacingSeconds` (default 1), and at least `confirmationSeconds`
(default 90) elapsed since the first matching current-failure observation. Closely
spaced status/cron polls do not count twice. The next available poll at or after the
window confirms a persistent fault; with the default one-minute cron the bound is
90 seconds plus one normal poll interval, assuming the provider API answers. A
backwards clock or changed witness resets the sequence. Both values are data, and
the spacing cannot exceed the confirmation window.

INFERRED from source (`src/index.js`, `runner.py`, `src/ledger.js`): confirmed fatal messages
close as `PROVIDER_FAULT` immediately. A provider row still pending at boot expiry
also qualifies, using a fresh provider reading. An API read failure alone does not
penalise a machine. A prolonged pre-report API outage, provider absence or a
provisioning timeout also permits replacement after cleanup, without hardware
penalties when there is no hardware-attribution evidence. The runner requests replacements in the same run/session;
only the trusted Worker authorizes them. It excludes every machine and host already
failed by that session, requires confirmed absence of the old instance, and
atomically checks cumulative spend, credit, daily and rolling caps before each
new create. A temporarily empty fresh market is polled within the original session window. Explicit create refusals have durable no-instance receipts and do not add hardware penalties; ambiguous
creates must reconcile before another instance can be admitted. A foreign initial
create response is not accepted as ownership proof: reconciliation locates the
owned label and unique startup script, and cleanup targets that exact instance only. Startup
scripts carry a per-rental nonce in a comment. New rentals require that unique
startup hash before initial adoption, including when a foreign row copies the
public label; legacy receipts retain their prior ownership checks. An owned instance whose label
changes can still prove its identity by that unique script hash. The provider image UUID is opaque, so a new proof does not compare it with a container image name; the legacy image check remains for old records.
An unresolved contradictory create id remains pending with liability; missing
label lookups alone cannot erase that uncertainty.

CODE READ: the default session allows eight create attempts, within 90 minutes
before the first accepted report. Attempts retain the original hard and soft
runtime deadlines, the original run allowance, and one authenticated log chain. The cumulative run allowance
is the smaller of the profile daily ceiling and its hourly price cap multiplied by
the original lease plus cleanup period; repeated transfer reservations cannot borrow
extra daily room above that run ceiling.
Legacy sessions with an already-used replacement and missing hardware history stop
with an explicit `LEGACY_HISTORY_MISSING` diagnostic; the Worker does not invent
the missing identities or reuse unknown failed hardware.
Every attempt has its own durable cost receipt; transfer allowances are charged
for each created attempt. Daily rental counts retain their existing per-check
semantics; spend includes all attempts.

INFERRED from source: on exhaustion the verdict remains `PROVIDER_FAULT` (runner/store
RETRY). Each public receipt names the machine, host, fixed reason and seconds
through teardown. The end time requires verified absence (INFERRED from source).
A log and a structured run report both establish the first-report boundary.
Any reported failure, including an old payload's provider-fault or unavailable
result, becomes FAIL and cannot authorize a replacement. A host Xid, guest FAIL,
harness failure or timeout after reporting never earns a provider penalty.
Persisted reported faults from older Workers also publish as FAIL.

CODE READ: spend here is the existing conservative compute/storage estimate plus
reserved transfer allowances, not a traffic meter or a real-time billing feed.
New rentals stop at their caps/deadlines. A provider outage can prevent timely
destruction or verification of absence: the Worker retains `cleanup_pending`,
the exact id and financial liability, retries cleanup on cron, and admits no
replacement for that session until absence is confirmed. No software assertion
can force deletion during a permanent provider outage or bound an unannounced
external price change. INFERRED from source: simulations restore failed APIs and assert verified cleanup
before ending each simulated run; they do not turn an unreachable provider into
proof of absence.

## Configuration

CODE READ: defaults are data in `defaults.json`; runtime overrides below are public
Worker variables validated by `src/policy.js`. Invalid policy values fail closed;
the existing identity-only teardown configuration still permits cleanup.

| Variable | Default | Accepted bound or meaning |
| --- | --- | --- |
| `PROVIDER_MAX_ATTEMPTS` | 8 | Integer 1–32 |
| `PROVIDER_SESSION_MINUTES` | 90 | 1–180 |
| `PROVIDER_CONFIRM_SECONDS` | 90 | 1–600 |
| `PROVIDER_OBSERVATION_SPACING_SECONDS` | 1 | 1–600, no greater than the confirmation window |
| `PROVIDER_PENALTY_BASE_USD_HOUR` | 0.25 | 0.001–10 |
| `PROVIDER_PENALTY_HALF_LIFE_HOURS` | 24 | 0.01–720 |
| `PROVIDER_PENALTY_CAP_USD_HOUR` | 5 | 0.001–100 |
| `PROVIDER_FATAL_SIGNATURES` | two rules | JSON list of literal match rules; at most 16 |
| `PROVIDER_IGNORABLE_SIGNATURES` | progress/retry literals | JSON list, at most 32 strings |
| `PINNED_MIDDLE_DIES` | AD102, AD103, AD104, AD106 | Ordered, unique known die ids; JSON `[]` restores rotation |

## Penalties and the middle gate

CODE READ (`src/ledger.js`, `src/offers.js`, `src/release.js`): provider faults
produce bounded `provider-machine:` and `provider-host:` histories, rather than
new permanent exclusions. Each fault stores a timestamp, reason, attempt identity
and escalation weight. Re-observing one attempt is idempotent. A fault's contribution
is `base * 2^(n-1) * 0.5^(age_hours/half_life_hours)`, with `n` its position when
appended to the retained history. The sum for machine plus host is capped at the
configured hourly-equivalent cap **after decay**, not before it. Ranking uses the
actual effective hourly quote plus that penalty; price and spend caps still use
the actual quote, so a penalised sole candidate remains eligible in a new session.
INFERRED from source: a verified PASS halves existing weights once per successful rental; no failure of
a test or harness adds a weight. Storage retains at most 32 faults per identity
and 256 machine/256 host histories, evicting the oldest identity deterministically.
Legacy timestamped provider-fault entries are migrated; explicit permanent entries,
untimestamped controls and configured machine/host exclusions remain exclusions.

CODE READ: only middle-tier check selection uses the ordered Ada pin. Within the
first pinned die that has acceptable offers within the remaining spending and credit room, country preference, effective price
plus penalty, and offer id determine the choice. Earlier exhausted pin entries
are recorded as `pin_skipped`; the reason is `PIN`. Only after every pinned die
has no acceptable offer does die family rotation run with `PIN_EXHAUSTED`. Provider
replacements try the pinned list again after excluding the session's failed
hardware. Release planning, small checks and nightly/sweep selection do not acquire
the middle pin. An empty list preserves the prior rotation behavior.

## Public rental facts

CODE READ (`src/rental.js`, `runner.py`, `public-log.js`, `gate_reuse.py`).
INFERRED formatting example with synthetic identities:

```text
RENTAL instance=900 host=3001 machine=100001 datacenter=true country=US ip=203.0.113.0 gpu=GTX_1660 price=0.820000
```

CODE READ: the Worker emits the line after creation, before a live contract refusal
can hide the attempt. The runner prints each rental attempt once. `country` is the
normalized offer geolocation, not text claimed by the box. `datacenter` is the
optional boolean offer field; a missing or malformed value is `unknown`.
The [official Vast query schema](https://docs.vast.ai/api/openapi.json) and
[official CLI fields](https://github.com/vast-ai/vast-python/blob/master/vast.py)
name `datacenter`; neither inspected schema establishes a datacenter-id response
field, so this implementation does not invent or print an id. The instance schema
names `public_ipaddr`; only its IPv4 network address at /24 or canonical IPv6 /48
prefix is retained in public metadata. IPv4-mapped IPv6 is normalized to IPv4.
CODE READ: the full address and SSH port are private control facts. The Worker
keeps at most 128 distinct addresses and 128 ports across replacement attempts;
if that bound is exceeded, text redaction conservatively masks all numeric/address
candidates. The authenticated status response carries an optional base64-encoded
`endpoint_redactions` JSON context (`ips`, `ports`, `overflow`) for the trusted
runner/renderer. Encoding is not an access boundary: this field is private,
validated before use, and never printed or copied into a public report.
CODE READ: every text output first masks by semantic value, independent of command
syntax: IPv4 dotted/zero-padded, integer/hex and mapped IPv6 aliases, and IPv6
compressed/expanded aliases. Every digit run numerically equal to a private SSH
port becomes `[redacted]`, including leading zeros and digits glued to option
letters. This privacy rule also masks unrelated numeric labels when their value
happens to equal that port. Counts, MACs and versions that do not match stay intact.
CODE READ: credential shapes and known secrets are removed before endpoint masking,
so a coincident port cannot split a credential and defeat its existing filter.
Raw box text is masked before sanitisation removes endpoint context,
then masked again after stripping, before the existing line/byte limits, on runner
output/summary, and at public log/report
publication. The runner retains context across attempts; the publication queue
snapshots it. A first report also learns endpoint facts from its live provider
inspection, without waiting for a status poll. Existing structural endpoint/SSH-option masking remains a backstop
for old records without provider endpoint facts, including clustered short options.
Reuse parsing accepts the new bounded
rental-line shape; runs with multiple rental lines remain ineligible for reuse
under its existing single-rental proof rule.

## GPU-free verification

CODE READ: `test/provider-market.test.js` runs 2,048 fixed Mulberry32 seeds by default
against the mocked Vast transport. Every simulated rental checks teardown, spend,
report boundaries, attempt/time limits, pin/penalty ranking, receipts and output
privacy. `test/provider-worst.test.js` drives the actual Worker paths through named
outages, ambiguous/foreign creates, fatal flaps, cap exhaustion, concurrency and
restart cases. `test/provider-values.test.js` covers the RV2 runtime regressions,
including recognized failure-state flaps at 30-second polls with a 90-second
confirmation window and a 75-minute boot limit,
1,024 fixed random command/log shapes through JS and both Python outputs, pending-state
permutations, message-change/recovery controls and bounded replacement history.
`test/provider-rules.test.js` contains independent controls;
`provider-session-mutants.mjs` requires at least two distinct assertion-failing
test names for every mutant, and rejects syntax/import/setup failures as killers.
`test/provider-mutants.test.js` runs that harness in ordinary Worker CI.

```sh
node --test ci/vast-broker/test/provider-{rules,session,worst,market,public}.test.js
node ci/vast-broker/provider-session-mutants.mjs
KF_PROVIDER_SIM_START=9001 KF_PROVIDER_SIM_SEEDS=5000 node --test ci/vast-broker/test/provider-market.test.js
```

CODE READ: simulations use no provider network, rentals, hardware or deployment.
Status protocol is 7 for the trusted metadata; box protocol stays 7. CODE READ: publication accepts canonical retry indices 0–31, validates the queue leg against its key, and publishes log/report identity from that same index. Offer evaluation snapshots one timestamp for every penalty decay calculation; input order and storage latency cannot age one candidate more than another.
