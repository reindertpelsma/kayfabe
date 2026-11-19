# The Worker's private-object presign, the lease-time trust scope, releaseSeq and the estimate as the price

STATUS: DESIGN-ONLY, 2026-11-17 (built and tested GPU-free; nothing deployed, no Cloudflare call, no secret, no box ran it). Owner rulings of 2026-11-17: private R2 objects are private only because their licences restrict redistribution, they are not sensitive; a valid run token is enough for the Worker to hand a runner a presigned GET URL for any object under a private prefix, after a trust, validity and tier check, with no per-object whitelist; boxes never get write access or credentials; proprietary images are for the largest (release) tier only; a run of untrusted provenance must never fetch a private object, and that is decided when the run token is issued. Follows `docs/design/V3_WORKER_PROPIMAGES.md` (#447). Files: `ci/vast-broker/private-objects.json`, `trust.json`, `src/privatedata.js`, `src/trust.js`, `src/legs.js`, edits in `src/index.js`, `src/routes.js`, `src/tokens.js`, `src/github.js`, `src/policy.js`, `src/sigv4.js`, `src/benchdata.js`, `defaults.json`; tests and mutants named at the end.

## Private is not sensitive

"Private" in this document means licence-restricted for redistribution, nothing more. The objects are not secret (an installer a vendor lets anyone download, an image built from licensed software). They sit in a private bucket so that the project does not redistribute them, and so that only runs the project trusts can pull them. The route therefore does not guard a secret; it guards a licence: untrusted code on a box must not be able to pull them, and the store does not trust such a run.

## The route

`GET /bench-data/<run>-<attempt>/private/<prefix>/<object path>` with `Authorization: Bearer <run token>`. It lives beside the public route: the public form `/bench-data/<run>/<name>` has no slash in a name, so the two never overlap, and the public allow-list (`src/benchdata.js`, six names, one lifetime of 20 minutes, four URLs per object) is byte-identical (a test pins the six keys). `<object path>` is one to five segments of `[A-Za-z0-9._-]`; a dot segment is never an object name; a query string is a 400.

Checks, in this order (the first failure answers; the body is `{state, reason, ...}` and the reason is one of the names below):

| Reason | Status | Meaning |
| --- | --- | --- |
| `TOKEN_INVALID` | 401 | no record, a token for another run, a bad signature, or the gateway could not verify the token |
| `TOKEN_EXPIRED` | 401 | the run is ended or closed, past its report expiry, or the provider does not report the instance running |
| `TRUST_SCOPE_MISSING` | 403 | the token or the record does not carry the `private` scope (below) |
| `TIER_NOT_ALLOWED` | 403 | the run's profile or source may not fetch private objects at all |
| `RATE_LIMITED` (reason `PRIVATE_URL_LIMIT`) | 429 | the run already received `urls_per_run` private URLs |
| `PREFIX_NOT_PRIVATE` | 403 | the first path segment is not one of the private prefixes |
| `OBJECT_MISSING` | 404 | the object does not exist (one presigned HEAD), or the path has a dot segment |

A store that is down or answers anything but 200 or 404 is `503 UNAVAILABLE` (`PRIVATE_STORE`); R2 settings that are missing fail closed with 503 and no redirect. Nothing in a response names another object: there is no listing, no pattern and no prefix enumeration; the only object name a caller ever sees is the one it sent. The success answer is a `302` with `Cache-Control: no-store`, whose `Location` is the presigned URL (host `<account>.r2.cloudflarestorage.com`, signed with the Worker's read-only key pair, `X-Amz-SignedHeaders=host`).

- **TTL**: `min(max_ttl_seconds, time left to the run's hard deadline)`; `max_ttl_seconds` is data (43200, 12 hours, and the data may only lower it: a configuration above 43200 is refused). A URL never outlives the run. A remainder below `min_ttl_seconds` is `TOKEN_EXPIRED`.
- **Limit**: `urls_per_run` (data, 24) private URLs per run, counted on the run record when a URL is signed; refused requests do not count; the count survives a Worker restart. The generic per-run request rate also applies.
- **Logs**: a mint logs one event `private_data_url` with `run`, `object` and `expires`; a refusal logs `private_data_refused` with `run`, the prefix and the reason. The URL is never logged, returned in a body, or stored in the ledger.
- **No write, no credential**: the Worker signs locally with the read-only key pair and sends one presigned HEAD (`expires` 60 s) to prove an object exists. A box receives a URL, never a key.
- **Public objects need no presign**: the public-bundle host serves them directly. The Worker never mints a URL for them, and a name under a non-private prefix is `PREFIX_NOT_PRIVATE`.

### Prefixes and tiers are data

`private-objects.json` (schema `kayfabe.worker-private-objects/1`) lists the prefixes: `windows-proprietary` (the objects `propimages.json` names), and `linux-proprietary`, `media-cache`, `specworkstation-4.0.0`, which the owner reserved and which hold no object named in this tree yet. A test checks that every object `propimages.json` names lies under a listed prefix and that none of the public allow-list's keys does. The `allow` rows say which runs may fetch private objects at all: profile `release` (every leg tier of a release; proprietary images are for the largest tier only), or source `admin` (an owner-started manual run). A GitHub check run (a pull request gate) and the sweep are `TIER_NOT_ALLOWED`; the validator refuses a configuration that allows the sweep, a bare GitHub source, or everything. Standalone runs that never call the Worker are unaffected: they skip private rows with their reason class, as before.

## Trust: the scope is decided at lease time from provenance the Worker checked

The scope is decided once, when the run token is issued, not at fetch time:

1. **Provenance** is built from facts the Worker has checked itself: for a GitHub run, the repository and head repository, head branch and event of the signed run attempt (`src/github.js` `authorize` already refuses a head repository that is not this repository and an actor that is not the owner), plus the signed OIDC claims; for an owner admin start, the owner's authenticated start of a commit that `github.reachable` accepted (reachable from master or next), stored on the batch. A request body and a pull request's tree can set none of it.
2. **`src/trust.js`** (`trustDecision`) maps provenance to `['public']` or `['public','private']` with the rules in `trust.json` (data): the repository must be this repository; the head repository must be this repository (a fork, a missing or null head is untrusted, a head that is not stated is treated as a fork); the event must be listed; the actor must be the owner; for GitHub the head branch must match a trusted branch (`next`, `master`, `claude/*`, `codex/*`), or be the head of a pull request from a branch of this repository (only a user with write access can push one); for an admin start the commit must be reachable. Anything missing or malformed is untrusted: fail closed. The three safety switches (`same_repository`, `require_owner_actor`, `admin_requires_reachable`) can only be on; a bare `*` branch is refused.
3. The decision is written into the **signed run token** (claim `scopes`) and into the **record** (`scopes`, and `scopeWhy`, a fixed token such as `UNTRUSTED:FORK_HEAD`). The private route reads both and requires both: a token without the claim (issued before it existed) or with a malformed one is `['public']`, and a token that claims `private` on a record decided public is refused, so neither a stale token nor a forged claim widens access.

Today every Worker run is already limited to the owner's repository (an owner-triggered run of this repository's pinned workflow); the scope makes the rule explicit and data-driven so that a future change which admits other provenance cannot silently reach the private store.

## releaseSeq and the instruction

`releaseSeq` (the ordinal of a release, whose parity rotates which die gets which image; a retry on the same sha must keep the assignment) comes from the ledger the Worker already has, with no new store: `release-seq:<sha>` (the number a commit's release got the first time) and `release-seq-next` (the number the next new sha gets). An owner start may pass `release_seq` (the number of the release tag) for the large tier; it wins for a new sha and moves the counter past it; a different request for a sha that already has a number is `RELEASE_SEQ_CONFLICT` (400). The same sha started again (a retry, a re-run, a later batch) reads its memo, so one sha is one assignment; consecutive new shas alternate parity. The batch stores `releaseSeq`; each leg carries it with the number of distinct dies of its family (`familyDies`).

When the owner sets `PROPIMAGES_ENABLED=true` (default off, a boolean public var; nothing changes while it is off), a release leg builds its instruction with `src/propimages.js` (`legInstruction` in `src/legs.js`), stores the canonical bytes and their SHA-256 on the run record, prices the leg from the estimate and logs the plan, the breakdown, the priced number and the hash (never a URL or an object name). The channel to the box is the instruction source the owner chose on 2026-11-15, the Worker with the run token: `GET /instruction/<run>-<attempt>` returns the canonical bytes (`application/json`, header `X-KF-Instruction-Sha256`), `404 NO_INSTRUCTION` for a run that has none. A leg whose instruction cannot be expressed (a driver-matrix leg: the instruction has no driver fields) is refused by name (`NOT_ELIGIBLE`, `INSTRUCTION_UNAVAILABLE`) when the flag is on, never rented without it.

## The estimate as the price

`resourceCaps` (`src/policy.js`) now takes `cfg.bandwidthEstimate = {downGb, upGb}` (`bandwidthEstimateInput(estimate)` in `src/bandwidth.js`). When present and valid (a finite number above zero, judged for down and up separately) the estimate REPLACES the profile number, so it can lower a leg's allowance as well as raise it, rounded up to whole GB and capped at the profile's hard maximum, the new data `payloadBandwidthMax` in `defaults.json` (a value that is missing or below the profile row is a configuration error). An invalid estimate is ignored: the profile number stays. The owner's `BANDWIDTH_*_GB` override still only raises (`max`). The result carries `estimate: {downGb, upGb, source, capped}` only when an estimate was given, so a leg with no estimate prices exactly as before. The maxima (150/180/260/100 GB down, 10 GB up, for small, middle, large, adhoc) are policy numbers chosen here at about twice the profile rows; they are not measured and the owner may change them. `policy.js` is one of the files the workflow bootstrap fetches from BASE: it adds no import.

## Image A's gate lane

The duplicate `edge` row is dropped from image A in `propimages.json`: Edge ships with Windows and the open app matrix already runs it (`edge_webgl` and the other Edge rows). The two golden instructions that listed it (`ad102-large-rehearsal`, `ad102-large-slot1-even`) were regenerated deliberately (the apps list lost `edge`); the parity tests against the box's parser and the standalone flags pass on the new files.

## Not done

- No deployment, Cloudflare change, secret or live Worker contact; the bundle pin in `deployment.json` and `wrangler.toml` is NOT changed: this change alters the Worker bundle (`index.js` and what it imports), and a local dry run on this machine does not reproduce the committed pin even for the unmodified source (it gives a different hash for `origin/next` and for the commit that wrote the pin), so a hash written here would be wrong. The deploy helper refuses a bundle that does not match its pin, so the pin must be regenerated in the reviewed deployment environment before any deploy.
- The box side: nothing fetches `/instruction` or the private URLs yet (the standalone run keeps its minted URL file). A box that wants the instruction asks for `/instruction/<run>` and treats `404 NO_INSTRUCTION` as none.
- No object exists yet under `linux-proprietary`, `media-cache` or `specworkstation-4.0.0`.
- Whether the release profile's `allow` row should also require the leg's instruction to list the object (a stricter per-leg rule) is not done: the owner ruled that no per-object list is needed.

## Tests and mutants

`test/private-presign.test.js` (route, refusals, limit, logs, no listing, public allow-list unchanged), `test/trust.test.js` (provenance table, lease-time scope in the token and record), `test/bandwidth-pricing.test.js` (the estimate as the price), `test/release-seq.test.js` (releaseSeq stability, instruction channel). Mutants, two named killers each: `ci/vast-broker/private-presign-mutants.mjs` (in `ci.yml`).
