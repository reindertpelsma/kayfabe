# Recorded manual mutation checks

[MEASURED] v22.22.1; explicit patches in `../mutants.js`. Every source file restored in finally; syntax checked before tests. No live APIs or GPU. Each mutant requires >=2 named failing tests.

| Mutation | Tests killed | Example killers |
| --- | ---: | --- |
| kill switch | 5 | bundle exercises the same refusal behavior; kill switch refuses rental false; kill switch refuses rental FALSE |
| owner numeric identity | 6 | run API must bind triggering_actor; nonowner push cannot spoof approval with owner name; nonowner push cannot spoof approval with owner email |
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
| SHA verdict cache | 3 | same SHA cache grants another run without rebilling; SHA cache does not rebill after gap expires false; SHA cache does not rebill after gap expires true |
| same SHA minimum gap | 2 | force rerun still requires minimum SHA gap; force gap survives restart |
| ledger-only operations | 4 | never inspect/delete unrecorded id 777; never inspect/delete unrecorded id 888; foreign run cannot read ledger instance |
| live label ownership | 2 | changed label prevents deletion; changed label prevents status log access |
| ambiguous-create binding | 2 | reconciliation never adopts labelled foreign instance with wrong script; reconciliation cannot adopt a changed image |
| TTL destruction | 2 | TTL cron destroys owned live instance; TTL cleanup survives restart and disabled renting |
| final-marker destruction | 7 | same SHA cache grants another run without rebilling; final PASS marker destroys immediately; final FAIL marker destroys immediately |
| SHA/nonce shell validation | 11 | onstart rejects SHA injection "$(id)"; onstart rejects SHA injection "`id`"; onstart rejects SHA injection "'" |
| log command sanitizer | 7 | sanitizer neutralizes command prefix ::error::hostile; sanitizer neutralizes command prefix ::set-output name=x::bad; sanitizer neutralizes command prefix \#\#[error]hostile |
| marker and nonce verdict | 9 | final FAIL marker destroys immediately; raw result rejects wrong-nonce; raw result rejects duplicate |
| fixed offer admission | 10 | invalid/expensive offer 0.51; invalid/expensive offer -1; invalid/expensive offer null |
| documented verification representation | 2 | documented Vast verification string is accepted without boolean alias; complete documented display offer remains within fixed capability |
| documented model representation | 2 | documented display GPU names map to the fixed model allowlist; complete documented display offer remains within fixed capability |
| documented offer pricing type | 63 | review_or_author accepts owner author; review mode accepts exact head approval by numeric owner; actor_only is explicitly selectable |
| poll permissions guard | 3 | extra workflow permission fails actions: write; extra workflow permission fails contents: write; extra workflow permission fails packages: write |
| attestation dependency guard | 2 | direct attestation dependency is rejected; transitive release dependency is rejected |
| must-fail control | 2 | box deliberate control exits failure on missing evidence; box harness accepts only a deliberately failed control |
| scheduled cleanup wiring | 2 | scheduled binding reaches Durable Object cleanup; bundled scheduled binding reaches Durable Object cleanup |
| public cron isolation | 2 | public request cannot invoke the internal cron route; bundled public request cannot invoke internal cron route |
