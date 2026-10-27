# UI1 v2 retained-log storage check

STATUS: LIVE local model and source analysis, 2026-10-26; no production storage queried.

INFERRED (policy/ledger analysis and `storage.jsonl`): recommend keeping `LOG_RETENTION_HOURS=6` at the current defaults. Forty-eight hours is allowed but has less headroom; it is not a blanket safe setting under the maximum configurable admission caps. No configuration is changed here.

CODE READ (`ci/vast-broker/src/policy.js`, `defaults.json`, `deploy.mjs`): retention accepts whole hours 1–48, defaults to six, and is allowed by the deployment helper. The sanitised full relay, including Worker lines, is capped at 4 MiB per run. The display tail is a separate 8 KiB string. The box callback cap is 2,000; Worker lines can fragment the relay further. Each nonempty append creates at least one log entry and one 64-character hash entry. KV keys, serialisation, record/result metadata and SQLite page/index overhead are additional storage.

CODE READ (`src/ledger.js`): retention starts at `ended`, not `started`. Open runs remain until cleanup settles them. Cron removes at most four ended records per tick, including log/hash chunks and the record. The one-minute cron has ample steady-state deletion capacity relative to the daily admission cap, but outages/backlogs remove any hard retention guarantee. Spend rows remain for 30 days; SHA indexes have their own horizon. The v2 viewer writes nothing.

INFERRED (transactional admission and daily accounting): with pruning keeping up, let `C` be the account concurrency cap and `D` the daily rental cap **per profile**, for the two profiles check/sweep. At the left edge of an H-hour window there are at most C open records; new records can start in at most `ceil(H/24)+1` UTC calendar budget buckets. Thus a conservative retained/open count is `C + 2*D*(ceil(H/24)+1)`. This includes old open records that end during the window. It does not include a backlog of overdue, unpruned records.

MEASURED (`storage.jsonl`, `storage.mjs`, two fresh Miniflare/workerd runs on 2026-10-26, identical results): the model stores exactly 4,194,304 log bytes, one hash per chunk, longest permitted run keys and an 8 KiB tail record through the SQLite-backed KV API. It reads the runtime's `storage.sql.databaseSize`. It does not model hosted quotas, PITR history, arbitrary prior database high-water marks, all result/index metadata or account-wide usage. It uses no production data or outbound network traffic.

| Chunks per run | SQLite bytes, each of two runs | Purpose |
|---:|---:|---|
| 256 | 4,399,104 | Full 16 KiB chunks |
| 2,000 | 9,650,176 | One chunk per callback at the callback cap |
| 131,072 | 32,169,984 | Conservative 32-byte fragmentation model, including hash-row overhead |

INFERRED (count bound and largest local model above): these are projections, not production measurements. Logical relay excludes all metadata; projected SQLite multiplies the largest modeled run by the count bound.

| Caps | Hours | Records | Logical relay MiB | Projected SQLite bytes |
|---|---:|---:|---:|---:|
| Default C=1, D=4/profile | 6 | 17 | 68 | 546,889,728 |
| Default C=1, D=4/profile | 48 | 25 | 100 | 804,249,600 |
| Maximum C=4, D=20/profile | 6 | 84 | 336 | 2,702,278,656 |
| Maximum C=4, D=20/profile | 48 | 124 | 496 | 3,989,078,016 |

CODE READ (Cloudflare documentation, 2026-10-26): the provider describes SQLite database size through [`databaseSize`](https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/#databasesize). Its [capacity FAQ](https://developers.cloudflare.com/durable-objects/reference/faq/#what-happens-when-a-durable-object-exceeds-its-storage-limit) names 10 GB per paid object and 1 GB on Free. Use the conservative 1 GB comparison until the coordinator confirms the actual plan; the account has separate shared limits. These are provider limits, not observations of this account.

INFERRED: six hours at the default caps leaves about 453 MB below that conservative 1 GB comparison in this deliberately fragmented model, before remaining metadata and other workload effects. Forty-eight hours leaves about 196 MB. Keep six hours as the operating recommendation and verify actual database size and pruning health before increasing it. Under maximum admission caps even the six-hour projection exceeds 1 GB; a shorter TTL alone cannot eliminate a two-calendar-day burst, so those caps need a plan/capacity change or lower report volume. No finite TTL protects against indefinitely stalled pruning or provides a strict physical-storage ceiling.

Reproduce locally after installing the broker's pinned dev dependencies:

```sh
MINIFLARE_ASSERT_BODIES_CONSUMED=true \
  node traces/vast_admin_v2_20261018/storage.mjs
```
