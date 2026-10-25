# Worker state audit

STATUS: LIVE, 2026-10-26. CODE READ at base 48c3b7d36; paths below are under `ci/vast-broker/src/`. This task changes no behavior.

## Literal module scope

| Binding | Location | Classification and two-isolate consequence |
| --- | --- | --- |
| `timingSafeEqual` | `tokens.js:27` | Platform function precomputation; mutable only through the explicit Node test seam. Each isolate binds the platform primitive independently. No request history. Explicit lint exception. |
| `tokenEncoder` | `tokens.js:3` | Stateless platform object precomputation. |
| `DENIED_NAMES` | `offers.js:24` | Mutable Map by type, populated once and never mutated. Pure country-name precomputation, identical in each isolate. |
| `INVISIBLE`, `OTHER_FORMAT` | `offers.js:9-10` | Global RegExp objects have mutable lastIndex; only passed to replace, which resets it. Pure normalization precomputation. |
| `MODELS` | `policy.js:22` | Empty legacy compatibility array, never mutated or consulted by production admission. |
| `DEFAULTS` and nested arrays/objects | `defaults.js:3` | Top level frozen, children mutable by type; read-only policy precomputation in production. Configuration builds fresh per-request data. |

CODE READ: `github.js` and `oidc.js` have no mutable module-level binding. The default export in `index.js:1260` calls `createGateway()` once, retaining the following closure state for an isolate. Literal module scope alone therefore misses relevant shared state.

## Gateway closure retained by the module default export

| Binding | Location | Classification and two-isolate consequence |
| --- | --- | --- |
| `verifier` / `OidcVerifier.keys` | `index.js:1118`, `oidc.js:15` | jose JWKS cache; signatures/claims verified each request. Separate isolates fetch keys independently; revocation visibility bounded by configured cache age. |
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

CODE READ: production requests route to one named BrokerLedger (`index.js:1112-1115`); its constructor owns createService (`index.js:1061`). `busy` and `queue` are **not** edge-isolate module globals today. Concurrent gateway isolates still reach one DO service. Task 2 must distinguish an existing storage-level invariant gap from a demonstrated production two-DO-isolate race. Test direct concurrent reserve calls and independent service closures against one real DO storage; also test the unmodified production gateway/DO route.

CODE READ: other source bindings are immutable scalar/function declarations, request-local variables, or per-record mutable data persisted through Ledger. No hidden module caches exist in policy.js. A source scan for top-level declarations, Map/Set allocations, instance properties and closure writes produced the inventory above.
