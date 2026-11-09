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
and the matching instance id; changes elsewhere in a boot log do not erase a
repeated fatal signature. Image pull progress, checksum verification, GPU
preparation, daemon retries and other unmatched error text do not qualify.
Fatal rules take precedence over ignorable progress on the same status line.
A recovery reading clears the sequence. Two distinct consecutive polls confirm
a fatal signature; the configurable confirmation window also permits confirmation
on a later observation. Duplicate readings at the same clock instant do not count
twice; a backwards clock resets the sequence.

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
The full address and SSH port are never public fields. Invalid address-like
text, including MAC addresses and version strings, keeps its original spelling. Output redaction covers
bare/bracketed endpoints, named SSH ports, command-line ports, provider diagnostics,
box log text, error paths and summaries. Reuse parsing accepts the new bounded
rental-line shape; runs with multiple rental lines remain ineligible for reuse
under its existing single-rental proof rule.

## GPU-free verification

CODE READ: `test/provider-market.test.js` runs 2,048 fixed Mulberry32 seeds by default
against the mocked Vast transport. Every simulated rental checks teardown, spend,
report boundaries, attempt/time limits, pin/penalty ranking, receipts and output
privacy. `test/provider-worst.test.js` drives the actual Worker paths through named
outages, ambiguous/foreign creates, fatal flaps, cap exhaustion, concurrency and
restart cases. `test/provider-rules.test.js` contains independent controls;
`provider-session-mutants.mjs` requires at least two distinct assertion-failing
test names for every mutant, and rejects syntax/import/setup failures as killers.
`test/provider-mutants.test.js` runs that harness in ordinary Worker CI.

```sh
node --test ci/vast-broker/test/provider-{rules,session,worst,market,public}.test.js
node ci/vast-broker/provider-session-mutants.mjs
KF_PROVIDER_SIM_START=9001 KF_PROVIDER_SIM_SEEDS=5000 node --test ci/vast-broker/test/provider-market.test.js
```

CODE READ: simulations use no provider network, rentals, hardware or deployment.
Status protocol is 7 for the new trusted metadata; box protocol stays 7.
