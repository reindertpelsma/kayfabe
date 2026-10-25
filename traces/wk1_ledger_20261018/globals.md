# Worker state audit

STATUS: LIVE, 2026-10-26. CODE READ at base 48c3b7d36; paths below are under `ci/vast-broker/src/`. This task changes no behavior.

## Literal module scope

| Binding (CODE READ at 48c3b7d36) | Location | Classification and two-isolate consequence |
| --- | --- | --- |
| `timingSafeEqual` | `tokens.js:27` | Platform function precomputation; mutable only through the explicit Node test seam. Each isolate binds the platform primitive independently. No request history. Explicit lint exception. |
| `tokenEncoder` | `tokens.js:3` | Stateless platform object precomputation. |
| `DENIED_NAMES` | `offers.js:24` | Mutable Map by type, populated once and never mutated. Pure country-name precomputation, identical in each isolate. |
| `INVISIBLE`, `OTHER_FORMAT` | `offers.js:9-10` | Global RegExp objects have mutable lastIndex; only passed to replace, which resets it. Pure normalization precomputation. |
| `MODELS` | `policy.js:22` | Empty legacy compatibility array, never mutated or consulted by production admission. |
| `DEFAULTS` and nested arrays/objects | `defaults.js:3` | Top level frozen, children mutable by type; read-only policy precomputation in production. Configuration builds fresh per-request data. |

CODE READ: `github.js` and `oidc.js` have no mutable module-level binding. The default export in `index.js:1260` calls `createGateway()` once, retaining the following closure state for an isolate. Literal module scope alone therefore misses relevant shared state.

## Gateway closure retained by the module default export

| Binding (CODE READ at 48c3b7d36) | Location | Classification and two-isolate consequence |
| --- | --- | --- |
| `verifier` / `OidcVerifier.keys` | `index.js:1118`, `oidc.js:15` | CODE READ: jose JWKS cache; signatures/claims checked each request. Separate isolates fetch keys independently; revocation visibility bounded by configured cache age. |
| `local` | `index.js:1119` | Map of loopback test issuers to JWKS verifiers; cache only, bounded at four. Production localIdentity rejects this mode. |
| `boxVerifier`, `boxSecret` | `index.js:1120` | Cache of RunTokens for current binding; verifies JWT anew; rebuilt when binding differs. |
| `boxWindow`, `boxCounts` | `index.js:1121-1122` | Decision-bearing front-door noise rate limiter. Two isolates each allow 300 callbacks/key/minute (and 1024 keys), so the configured front-door threshold is not an account-wide cap. DO rechecks record token, expiry and report quotas. |
| `controlWindow`, `controlCount`, `controlIps` | `index.js:1123-1125` | Decision-bearing front-door noise rate limiter. Alternating isolates multiplies per-IP/global allowances; cold starts reset them. DO authorization and spend limits must remain authoritative. |

## Service closure retained only inside the singleton Durable Object

| Binding | Location | Classification and failure if incorrectly moved to multiple services |
| --- | --- | --- |
| `ledger` | `index.js:71` | Durable storage facade; no cached ledger state. |
| `verifier`, `localVerifier` | `index.js:72,77` | JWKS/verifier caches; per-request signature/claims validation. |
| `tokens` | `index.js:73` | Signing configuration precomputation; record authorization checked each time. |
| `githubCache` | `index.js:74`, `github.js:9,21-29,50-53` | Immutable SHA/report source cache and explicitly cached lifecycle run reads. Admission run lookup stays fresh. Two services can see lifecycle status up to 60 s apart, delaying cancellation cleanup; absolute DO deadlines still apply. Cache, not admission authority. |
| `offers.cached`, `offers.at`, `offers.failed` | `index.js:75`, `offers.js:122-124` | Market cache (60 s), including failure cooldown. Each evaluation rereads bad-machine and open-machine indexes from ledger. Separate isolates can choose the same stale available machine; final reservation must recheck it. |
| `busy` | `index.js:76` | Decision-bearing DO-local overlap rejection. Separate services each see false and both start rent. No account-wide guarantee without durable admission. |
| `admissionWindow`, `admissionCount` | `index.js:78-79` | Decision-bearing DO-local request throttle. Multiple independent services multiply the 30/minute allowance. Not a spend cap. |
| `executing`, `queue` | `index.js:911-912` | Decision-bearing DO-local scheduler. Independent queues interleave read/check/write admission and lifecycle updates; duplicate keys, SHA reuse, slot and budget overruns become possible without transactional reservation. |

CODE READ at 48c3b7d36: requests route to one named BrokerLedger (`index.js:1112-1115`); its constructor owns createService (`index.js:1061`). `busy` and `queue` are **not** edge-isolate module globals today. Concurrent gateway isolates still reach one DO service. Task 2 must distinguish an existing storage-level invariant gap from a demonstrated production two-DO-isolate race. Test direct concurrent reserve calls and independent service closures against one real DO storage; also test the unmodified production gateway/DO route.

CODE READ: other source bindings are immutable scalar/function declarations, request-local variables, or per-record mutable data persisted through Ledger. No hidden module caches exist in policy.js. A source scan for top-level declarations, Map/Set allocations, instance properties and closure writes produced the inventory above.

## Async interleaving addendum (owner 2026-10-26)

CODE READ at 48c3b7d36: any await permits same-isolate interleaving. Inventory of retained-state read/modify/write spanning awaits:

- `index.js:996-1008`: busy is set before awaiting serial/perform and cleared in finally. Removing this guard allows concurrent services to pass admission reads. It is not an admission lock.
- `index.js:927-941`: executing and queue stay live across `await task.action()`. Other requests push/sort while drain waits. The single-instance scheduler currently protects lifecycle read/modify/write, not merely performance. Production must use an explicit DO input gate before the queue can be described as optional.
- `index.js:748-852`: cost/SHA/open/cap/budget reads precede awaits on protocol, offers, credit, JWT and hashing, then reserve writes without rechecking. These decisions must move inside the reservation transaction. `offers.js:132-137` also checks bad/reserved machines before those awaits.
- `index.js:1170-1180`: cached boxVerifier is selected before awaiting JWT verification; a simultaneous binding refresh may replace the cache. Verify uses a local receiver captured at invocation; no authorization result is cached. Counter updates after the await are synchronous and can only overcount the current minute at a window boundary; cold isolates multiply the best-effort noise allowance.
- `github.js:21-53`: cache miss/read, await network, eviction/write. Concurrent responses can replace newer lifecycle cache values with older observations. Immutable SHA reads are safe; lifecycle state must never authorize new spend. Cache writes are a best-effort optimization.
- `oidc.js:15-22,27`: jose owns an asynchronous JWKS cache and pending-fetch state. Every call still verifies signature, issuer/audience/time. The pending operation must not become request authority; token claims are never cached.
- `offers.js:127-132`: timestamp, clear/failed state, await offers, then cache/failure writes. A second concurrent evaluate can refuse during a successful in-flight fill or overwrite a later fetch; only cache availability is affected. Bad/reserved machines are reread from durable storage and must be rechecked at reserve.
- `tokens.js:27-39`: platform comparator is read before and after await digest. A test-only replacement during hashing could change the chosen function. Capture the primitive synchronously per call, explicitly except the synchronous test setter from lint.
- `policy.js`: no retained mutable cache; configuration objects and temporary Sets are rebuilt synchronously per request. `DEFAULTS` is read-only source data.

CODE READ: report, kill, stop, cron and blob counters also load durable records and save after awaits (`index.js:184-389,402-537`, `blobs.js`). The current queue protects these paths only in one service instance. The reservation transaction does not by itself make every lifecycle operation concurrent-safe; a DO input gate is needed if the scheduler is removed or treated as optional. Gateway window/counter changes (`index.js:1135-1151`) contain no intervening await and are best-effort only.

## Final scope and module instances

CODE READ at 48c3b7d36: `export default createGateway()` constructs a behavior closure at module scope; the gateway instance and its rate/verifier state are listed above. There is no module-scope `new Github`, `new OidcVerifier`, `new Vast`, `new RunTokens` or `new OfferBook`, and no static class field or method in src/. Constructors are invoked inside service/gateway factories. `new TextEncoder()` is a stateless precomputation; DENIED_NAMES is read-only after construction.

Owner scope cut, 2026-10-26: audit only, plus the reservation transaction and one workerd concurrency test. No broad request reconstruction, lint, admin UI or counter/cache refactor. The earlier gateway and service interleaving observations remain audit findings, not claims that they were fixed here.

Owner guideline: reconstruct request handlers from env and durable state inside fetch/scheduled; module state is only constants or synchronously computed constant caches. Static utility methods may not write globals; static fields are constants. Every await may expose changed shared state; the Durable Object is the only lock.
