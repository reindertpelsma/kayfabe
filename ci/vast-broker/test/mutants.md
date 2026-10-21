# Recorded manual mutation checks

[MEASURED: 2026-10-21 mutation campaign] v22.22.1; explicit patches in `../mutants.js`. Every source file restored in finally; syntax checked before tests. No live APIs or GPU. Each mutant requires >=2 named failing tests.

| Mutation | Tests killed | Example killers |
| --- | ---: | --- |
| kill switch | 5 | bundle exercises the same refusal behavior; kill switch refuses rental false; kill switch refuses rental FALSE |
| owner numeric identity | 7 | run API must bind triggering_actor; nonowner push cannot spoof approval with owner name; nonowner push cannot spoof approval with owner email |
| review head SHA | 2 | review_only rejects old-head; review cannot authorize a nonowner push at a newer head |
| draft refusal | 2 | draft owner push does not rent; draft review does not override default refusal |
| OIDC claims | 5 | unauthorized token cannot rent {"aud":"wrong"}; unauthorized token cannot rent {"iss":"wrong"}; unauthorized token cannot rent {"exp":1800000000} |
| credit reserve | 6 | credit guard reserves headroom 0; credit guard reserves headroom 7.99; credit guard reserves headroom 8.01 |
| daily rental count | 2 | budget prevents admission rentals; daily count remains across restart |
| daily spend | 2 | budget prevents admission daily; budget survives restart daily |
| rolling 30-day spend | 3 | budget prevents admission rolling; budget survives restart rolling; rolling cap counts a previous UTC day |
| one live instance | 6 | one-live blocks another run; one-live survives restart; superseding push destroys old instance and returns BUSY false |
| superseding push | 2 | superseding push destroys old instance and returns BUSY false; superseding push destroys old instance and returns BUSY true |
| run idempotence | 6 | run attempt idempotence before destroy; run attempt idempotence after restart; ambiguous creation reconciles exact prior intent without second create |
| SHA verdict cache | 5 | same SHA cache grants another run without rebilling; SHA cache does not rebill after gap expires false; SHA cache does not rebill after gap expires true |
| same SHA minimum gap | 2 | force rerun still requires minimum SHA gap; force gap survives restart |
| ledger-only operations | 4 | never inspect/delete unrecorded id 777; never inspect/delete unrecorded id 888; foreign run cannot read ledger instance |
| live label ownership | 2 | changed label prevents deletion; changed label prevents status log access |
| ambiguous-create binding | 2 | reconciliation never adopts labelled foreign instance with wrong script; reconciliation cannot adopt a changed image |
| TTL destruction | 2 | TTL cron destroys owned live instance; TTL cleanup survives restart and disabled renting |
| final-marker destruction | 9 | same SHA cache grants another run without rebilling; final PASS marker destroys immediately; final FAIL marker destroys immediately |
| SHA/nonce shell validation | 11 | onstart rejects SHA injection "$(id)"; onstart rejects SHA injection "`id`"; onstart rejects SHA injection "'" |
| log command sanitizer | 14 | sanitizer neutralizes command prefix ::error::hostile; sanitizer neutralizes command prefix ::set-output name=x::bad; sanitizer neutralizes command prefix \#\#[error]hostile |
| marker and nonce verdict | 9 | final FAIL marker destroys immediately; raw result rejects wrong-nonce; raw result rejects duplicate |
| fixed offer admission | 10 | invalid/expensive offer 0.51; invalid/expensive offer -1; invalid/expensive offer null |
| documented verification representation | 2 | documented Vast verification string is accepted without boolean alias; complete documented display offer remains within fixed capability |
| documented model representation | 2 | documented display GPU names map to the fixed model allowlist; complete documented display offer remains within fixed capability |
| documented offer pricing type | 105 | review_or_author accepts owner author; review mode accepts exact head approval by numeric owner; actor_only is explicitly selectable |
| poll permissions guard | 3 | extra workflow permission fails actions: write; extra workflow permission fails contents: write; extra workflow permission fails packages: write |
| attestation dependency guard | 2 | direct attestation dependency is rejected; transitive release dependency is rejected |
| cold-box disk guard | 2 | cold-box disk preflight 22000000000; cold-box disk preflight 59999999999 |
| must-fail control | 2 | box deliberate control exits failure on missing evidence; box harness accepts only a deliberately failed control |
| scheduled cleanup wiring | 2 | scheduled binding reaches Durable Object cleanup; bundled scheduled binding reaches Durable Object cleanup |
| Unicode format sanitizer | 2 | Unicode format control is removed before command neutralization 8203; Unicode format control is removed before command neutralization 8238 |
| excerpt command prefix | 2 | excerpt tail cannot introduce a bare workflow command ::error::; excerpt tail cannot introduce a bare workflow command \#\#[error] |
| unsupported job syntax | 2 | flow-style release dependency cannot be silently ignored; quoted release job cannot bypass structural parsing |
| distinct client refusal | 3 | workflow client exposes budget refusal as a distinct failing state; workflow client exposes missing approval as a distinct failing state; NOT_NEEDED is explicit successful job and commit status |
| terminal client completion | 2 | workflow client exits on a terminal unavailable verdict without another poll; workflow client exits when a pending run completes unavailable |
| public cron isolation | 2 | public request cannot invoke the internal cron route; bundled public request cannot invoke internal cron route |
| automatic base allowlist | 3 | automatic non-master base is NOT_ELIGIBLE integration/candidate; automatic non-master base is NOT_ELIGIBLE another-pr; missing API base never trusts an OIDC base claim |
| docs-only exclusion result | 4 | complete exclusion list is NOT_NEEDED README.md; complete exclusion list is NOT_NEEDED docs/design.md,traces/evidence.txt,archive/old.rs,LICENSE-MIT,.github/ISSUE_TEMPLATE/bug.yml; complete exclusion list is NOT_NEEDED docs/guide.md,nested/notes.md |
| unknown file-list skip | 5 | incomplete or unknown file metadata runs empty; incomplete or unknown file metadata runs missing-count; incomplete or unknown file metadata runs truncated |
| canonical path validation | 12 | unlisted path runs despite adjacent docs ci/vast-broker/README.md; unlisted path runs despite adjacent docs ci/vast-broker/test/example.md; rename checks both paths docs/new.md from docs/../src/code.md |
| renamed previous path | 2 | rename checks both paths docs/new.md from src/code.rs; rename checks both paths docs/new.md from docs/../src/code.md |
| cached PR display grant | 2 | same SHA cache names the requesting PR in rent; same SHA cache names the requesting PR in status |
| request bearer transport | 4 | request GitHub bearer used only for read-only GitHub calls /rent; request GitHub bearer used only for read-only GitHub calls /status/900; new request replaces prior bearer and cron retains none |
| request bearer retention | 3 | request GitHub bearer used only for read-only GitHub calls /rent; new request replaces prior bearer and cron retains none; request token absent from responses and ledger box output |
| public GitHub fallback | 2 | unusable read bearer falls back to authenticated public facts 401; unusable read bearer falls back to authenticated public facts 403 |
| owner branch without PR | 2 | owner dispatch permits branch without a PR owner-pr; owner dispatch permits branch without a PR integration/test |
| immutable API cache | 2 | static GitHub facts have a short token-free cache; static response cache works across request clients without retaining bearers |
| mandatory status writer jobs | 2 | missing status writer job fails the guard pending; missing status writer job fails the guard report |
| token-shaped log redaction | 2 | sanitizer redacts token-shaped text without retaining a request ghs_; sanitizer redacts token-shaped text without retaining a request gith |
| isolated writer permissions | 2 | status writer cannot gain content permission pending; status writer cannot gain content permission report |
