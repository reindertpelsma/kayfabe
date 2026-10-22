# Q3 Energy rate table

MEASURED at rev 468a35c9 (run4301) and the per-run revisions in table.json: runs4301–4307, exact revisions and first-event/channel facts in table.json and per-run channels.json; 7/11 trials complete.

| Build | Q3 pass/fail | Prior standalone pass/fail | Combined fail/n | Q3 exported guest scores |
|---|---:|---:|---:|---|
| D | 2/0 | 0/0 | 0/2 | 36.4, 39.87 |
| A | 2/0 | 0/2 | 2/4 | 41.19, 39.29 |
| B | 2/0 | 1/0 | 0/3 | 45.35, 38.9 |
| C | 0/1 | 1/1 | 2/3 |  |

CODE READ: D = descriptor-only468a35c9; A = descriptor+SA-02 40583bc2; B = descriptor+batched-walk a83394e6; C = candidate afe00b3c. Prior standalone runs are A4263/4266, B4264, C4262/4267. K1 mixed3454 and candidate mixed4260 are reference observations only.

| Order/run | Build | Host start CEST | Result | First153 guest | Host detection CEST | Failing forwarded/GP |
|---|---|---|---|---|---|---|
| 1/run4301 | D1 | 11:17:51 | PASS | none | none | none |
| 2/run4302 | A3 | 11:22:49 | PASS | none | none | none |
| 3/run4303 | B2 | 11:27:45 | PASS | none | none | none |
| 4/run4304 | C3 | 11:32:40 | FAIL | 2026-10-22T09:36:16.5823597+02:00 | 2026-10-22T11:36:30.596441+02:00 | 35/34 |
| 5/run4305 | D2 | 11:37:01 | PASS | none | none | none |
| 6/run4306 | B3 | 11:42:02 | PASS | none | none | none |
| 7/run4307 | A4 | 11:47:01 | PASS | none | none | none |

MEASURED at rev 468a35c9, run4301 (4301/channels.json): run-order.csv and per-run channels.json retain the guest clock and host clock separately; run4301 guest captured09:18 while host captured11:18 CEST, approximately two hours apart. Times are not silently converted.

CODE READ: production logs expose RmAlloc seq=0 context, CHAN-FACTS and token/host birth linkage; they do not identify a unique complete RPC transaction replay. Full logs and CoWs stay under /data/codex-Q3/windows-20261022/ and /data/codex-Q3/energy-bisect/.
