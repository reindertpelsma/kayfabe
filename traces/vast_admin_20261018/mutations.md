| Mutant | Killing tests | Two independent witnesses |
|---|---:|---|
| M23 JWT signature verification bypass | 13 | admin UI rejects invalid Access claims /api/list aud; admin UI rejects invalid Access claims /api/list iss |
| M24 truncated ledger accepted | 2 | admin list refuses truncated ledger at 1000 records; admin list refuses truncated ledger at 1001 records |
| M01 internal origin bypass | 2 | admin DO rejects a public origin for list; admin DO rejects HTTP internal hostname for list |
| M02 public routing exposed | 4 | admin public gateway hides list; admin public gateway hides prefixed list |
| M03 raw record disclosure | 2 | admin list includes active and ended runs, newest first, exact whitelist; admin list caps at 100 newest records and excludes nested/non-scalar values |
| M04 scalar whitelist bypass | 2 | admin list includes active and ended runs, newest first, exact whitelist; admin list caps at 100 newest records and excludes nested/non-scalar values |
| M05 oldest first | 3 | admin list includes active and ended runs, newest first, exact whitelist; admin list caps at 100 newest records and excludes nested/non-scalar values |
| M06 output cap removed | 2 | admin list caps at 100 newest records and excludes nested/non-scalar values; admin list cap still holds with ended runs and missing optional fields |
| M07 ambiguous selection accepted in DO | 2 | admin stop rejects malformed selection {"all":true,"run":"123-1"}; admin stop rejects malformed selection {"run":"123-1","extra":true} |
| M08 stop bypasses owner service | 8 | admin stop forwards exactly once to owner route: one; admin stop preserves actual owner cleanup and settlement: one |
| M09 failed cleanup reported as success | 4 | admin stop cannot settle when provider deletion fails: one; admin stop retains liability after DELETE acknowledgement with instance present: one |
| M10 owner authority omitted | 8 | admin stop forwards exactly once to owner route: one; admin stop preserves actual owner cleanup and settlement: one |
| M11 deletion acknowledgement treated as absence | 2 | admin stop retains liability after DELETE acknowledgement with instance present: one; admin stop retains liability after DELETE acknowledgement with instance present: all |
| M12 owner check bypass | 5 | admin UI valid signature with wrong email is forbidden /; admin UI valid signature with wrong email is forbidden /api/list |
| M13 issuer check omitted | 2 | admin UI rejects invalid Access claims /api/list iss; admin UI rejects invalid Access claims /api/stop iss |
| M14 audience check omitted | 2 | admin UI rejects invalid Access claims /api/list aud; admin UI rejects invalid Access claims /api/stop aud |
| M15 time validation weakened | 4 | admin UI rejects invalid Access claims /api/list exp; admin UI rejects invalid Access claims /api/list nbf |
| M16 empty audience not fail-closed | 2 | admin UI unconfigured policy /api/list {"ACCESS_AUDIENCE":""}; admin UI unconfigured policy /api/stop {"ACCESS_AUDIENCE":""} |
| M17 ambiguous selection accepted in UI | 2 | admin UI rejects invalid stop payload {"all":true,"run":"123-1"}; admin UI rejects invalid stop payload {"run":"123-1","extra":true} |
| M18 proxy paths swapped | 4 | admin UI list forwards to named DO once and returns list shape; admin UI stop calls the DO exactly once {"run":"123-1"} |
| M19 duplicate stop proxy call | 2 | admin UI stop calls the DO exactly once {"run":"123-1"}; admin UI stop calls the DO exactly once {"all":true} |
| M20 Stop all confirmation bypass | 3 | admin UI page is plain table with timed refresh and stop controls; admin UI page renders safe cells and Stop only for active runs |
| M21 ended run gets Stop button | 2 | admin UI page renders safe cells and Stop only for active runs; admin UI Stop all cancels without request and confirms one stop |
| M22 untrusted cells interpreted as HTML | 3 | admin UI page is plain table with timed refresh and stop controls; admin UI page renders safe cells and Stop only for active runs |
