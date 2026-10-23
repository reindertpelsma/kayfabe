# Recorded manual mutation checks

[MEASURED: 2026-10-22 endpoint secrecy plus Amendment 3/3b mutation campaign] v22.22.1; explicit patches in `../mutants.js`. Every source file restored in finally; syntax checked before tests. Worker mutations run all source-facing test files explicitly in parallel; workflow mutations also run shell-client fixtures. Those fixtures read YAML only and never import Worker source. No live APIs or GPU. Each mutant requires >=2 named failing tests.

| Mutation | Tests killed | Example killers |
| --- | ---: | --- |
| inherited endpoint secrets | 7 | unapproved reusable job cannot inherit endpoint secrets run; unapproved reusable job cannot inherit endpoint secrets utility; unsupported scalar forwarding is rejected "secrets: &all inherit" |
| endpoint aliases in step metadata | 2 | broker step name cannot reference endpoint alias WORKER_URL; broker step name cannot reference endpoint alias WORKER_PATH |
| optional prefix admission | 4 | wrong or encoded prefix fails before auth wrong-xx; wrong or encoded prefix fails before auth short; wrong or encoded prefix fails before auth unit-pat |
| native constant-time prefix comparison | 2 | prefix compares through native crypto 0; prefix compares through native crypto 42 |
| public prefix gate | 3 | public routing rejects without forwarding /rent; public routing rejects without forwarding /stop-all; public routing forwards a correctly prefixed request unchanged |
| ledger service prefix gate | 29 | unprefixed endpoint has generic 404 /rent; unprefixed endpoint has generic 404 /status/123-1; unprefixed endpoint has generic 404 /report/123-1 |
| prefix charset and bounds | 6 | invalid prefix config fails closed 5; invalid prefix config fails closed 129; onstart rejects prefix interpolation 7 |
| fixed callback prefix | 3 | actual fixed Python sender backs up hostile output and reports exit status 0; actual fixed Python sender backs up hostile output and reports exit status 7; fixed callback URL includes the exact optional prefix |
| endpoint text redaction | 5 | box log cannot reflect known endpoint component https://; box log cannot reflect known endpoint component broker.e; box log cannot reflect known endpoint component broker |
| Actions secret instead of variable | 7 | actual broker uses Actions secrets only inside its execution step; canonical broker step secret bindings pass the guard; explicit unrelated reusable secret mapping remains allowed |
| workflow API prefix | 3 | secret API base includes prefix while audience stays exact origin ""; secret API base includes prefix while audience stays exact origin "/"; derived masks exist before the first request 47 |
| derived hostname masks | 2 | derived masks exist before the first request 0; derived masks exist before the first request 47 |
| encoded audience mask | 2 | derived masks exist before the first request 0; derived masks exist before the first request 47 |
| shell tracing disabled | 2 | shell tracing cannot reveal the endpoint 0; shell tracing cannot reveal the endpoint 47 |
| transport diagnostic suppression | 2 | transport failures never print endpoint parts rent; transport failures never print endpoint parts status |
| secret context placement guard | 12 | another job cannot receive URL secret pending; another job cannot receive URL secret report; another job cannot receive URL secret run |
| secret object access guard | 2 | opaque secret object access fails toJSON(secrets); opaque secret object access fails secrets[inputs.name] |
| secret alias logging guard | 2 | broker cannot echo endpoint alias WORKER_URL; broker cannot echo endpoint alias WORKER_PATH |
| secret job output guard | 2 | broker job output cannot carry endpoint url; broker job output cannot carry endpoint path |
| secret alias job isolation | 2 | endpoint alias cannot enter another job WORKER_URL; endpoint alias cannot enter another job WORKER_PATH |
| literal run and renderer enclosure | 4 | unsupported broker run style fails closed inline WORKER_URL; unsupported broker run style fails closed inline WORKER_PATH; unsupported broker run style fails closed folded WORKER_URL |
| HMAC tag and commit binding | 2 | HMAC binds immutable ledger field sha; HMAC binds immutable ledger field label |
| closed ledger rejects reports | 2 | stop revokes even while Vast delete fails but GET stays live oidc; stop revokes even while Vast delete fails but GET stays live owner |
| actual Vast liveness | 4 | report closes and rejects non-live rental stopped; report closes and rejects non-live rental gone; report closes and rejects non-live rental exited |
| liveness cache expiration | 2 | positive liveness cache expires in seconds and fails closed exited; positive liveness cache expires in seconds and fails closed api-error |
| report count cap | 2 | report count cap survives retries and restart false; report count cap survives retries and restart true |
| cumulative report byte cap | 2 | per-run input byte cap is persistent false; per-run input byte cap is persistent true |
| streamed request byte cap | 2 | streaming body limit rejects bytes before parse 65537; streaming body limit rejects bytes before parse 131072 |
| report rate cap | 3 | per-run rate cap counts authenticated retries false; per-run rate cap counts authenticated retries true; V4 kill shares report rate limiter and cannot extend grace |
| preparse JSON depth gate | 2 | depth rejected before native JSON parser array; depth rejected before native JSON parser object |
| strict report schema | 6 | strict report schema rejects {"seq":"1","kind":"log","text":"x"}; strict report schema rejects {"seq":1.5,"kind":"log","text":"x"}; strict report schema rejects {"seq":0,"kind":"log","text":"x"} |
| strict result schema | 2 | strict report schema rejects {"seq":1,"kind":"result","result":{"nonce":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","verdict":"SUCCESS"}}; strict report schema rejects {"seq":1,"kind":"result","result":{"nonce":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","verdict":"PASS","extra":true}} |
| report sequence immutability | 3 | same seq identical payload is idempotent and gaps are allowed; changed duplicate or older seq cannot overwrite 1; changed duplicate or older seq cannot overwrite 2 |
| structured result nonce | 2 | structured result rejects a different valid nonce c; structured result rejects a different valid nonce d |
| free text cannot decide verdict | 3 | free text cannot set verdict KF_RESULT bbbbbbbbbb; free text cannot set verdict {"seq":2,"kind":"res; free text cannot set verdict ::set-output name=ve |
| structured result owned cleanup | 16 | structured PASS result destroys at grace expiry; structured FAIL result destroys at grace expiry; local HTTP integration: owner rent to authenticated reports to automatic destroy |
| native owner secret comparison | 13 | foreign run cannot destroy ledger instance; native empty POST bad bearer is unauthorized /stop-all; native empty POST bad bearer is unauthorized /stop/123-1 |
| owner-only stop-all | 3 | native empty POST bad bearer is unauthorized /stop-all; box token has no stop capability stop-all; OIDC cannot call owner stop-all |
| OIDC run-scoped status and stop | 4 | foreign run cannot read ledger instance; foreign run cannot destroy ledger instance; wrong stop auth cannot touch a run {"claims":{"run_id":"124"}} |
| ledger-only owner stop | 4 | never inspect/delete unrecorded id 777; never inspect/delete unrecorded id 888; owner stop never adopts unrecorded labelled instance 888-1 |
| stop-all cannot adopt account entries | 4 | correct prefix reaches protected stop-all; owner stop-all destroys ledger instances only and remains idempotent; stop-all handles every open ledger record and no foreign account entries |
| revocation precedes failed delete | 15 | report closes and rejects non-live rental expired; report closes and rejects non-live rental stopped; report closes and rejects non-live rental gone |
| all log lines prefixed | 36 | local HTTP integration: owner rent to authenticated reports to automatic destroy; sanitizer prefixes every command-like line ::error::hostile; sanitizer prefixes every command-like line ::set-output name=x::bad |
| embedded command delimiters | 13 | local HTTP integration: owner rent to authenticated reports to automatic destroy; sanitizer prefixes every command-like line ::error::hostile; sanitizer prefixes every command-like line ::set-output name=x::bad |
| per-line UTF8 byte cap | 3 | tail line/count/bytes are capped with dropped-byte accounting; multibyte display line is capped by UTF8 bytes 233; multibyte display line is capped by UTF8 bytes 128512 |
| log line count cap | 3 | sanitizer bounds huge lines or line counts 5000; report tail keeps at most 200 short lines 501; report tail keeps at most 200 short lines 1000 |
| log tail byte cap | 3 | tail line/count/bytes are capped with dropped-byte accounting; V4 bounded log cap counts dropped rendered bytes; V4 smaller full log cap bounds prefix expansion |
| orphan index cannot block cleanup | 2 | incomplete open index cannot block known owned cleanup cron; incomplete open index cannot block known owned cleanup stop-all |
| kill switch | 6 | kill switch refuses rental false; kill switch refuses rental FALSE; kill switch refuses rental  |
| owner numeric identity | 7 | run API must bind triggering_actor; nonowner push cannot spoof approval with owner name; nonowner push cannot spoof approval with owner email |
| review head SHA | 2 | review_only rejects old-head; review cannot authorize a nonowner push at a newer head |
| draft refusal | 2 | draft owner push does not rent; draft review does not override default refusal |
| OIDC claims | 11 | unauthorized token cannot rent {"aud":"wrong"}; unauthorized token cannot rent {"iss":"wrong"}; unauthorized token cannot rent {"exp":1800000000} |
| credit reserve | 18 | credit guard reserves headroom 0; credit guard reserves headroom 7.99; credit guard reserves headroom 8.01 |
| daily rental count | 2 | budget prevents admission rentals; daily count remains across restart |
| daily spend | 2 | budget prevents admission daily; budget survives restart daily |
| rolling 30-day spend | 3 | budget prevents admission rolling; budget survives restart rolling; rolling cap counts a previous UTC day |
| one live instance | 6 | one-live blocks another run; one-live survives restart; superseding push destroys old instance and returns BUSY false |
| superseding push | 2 | superseding push destroys old instance and returns BUSY false; superseding push destroys old instance and returns BUSY true |
| run idempotence | 6 | run attempt idempotence before destroy; run attempt idempotence after restart; ambiguous creation reconciles exact prior intent without second create |
| SHA verdict cache | 5 | same SHA cache grants another run without rebilling; SHA cache does not rebill after gap expires false; SHA cache does not rebill after gap expires true |
| same SHA minimum gap | 2 | force rerun still requires minimum SHA gap; force gap survives restart |
| live label ownership | 4 | changed label prevents deletion; changed label prevents status log access; absence does not accept malformed or foreign {"id":900,"label":"foreign"} |
| ambiguous-create binding | 2 | reconciliation never adopts labelled foreign instance with wrong script; reconciliation cannot adopt a changed image |
| TTL destruction | 2 | V4 accepted callback ends provisioning deadline; V4 hard TTL survives restart with accepted progress and no result |
| SHA/nonce shell validation | 11 | onstart rejects SHA injection "$(id)"; onstart rejects SHA injection "`id`"; onstart rejects SHA injection "'" |
| callback shell validation | 2 | onstart rejects callback interpolation run; onstart rejects callback interpolation token |
| fixed offer admission | 10 | invalid/expensive offer 0.51; invalid/expensive offer -1; invalid/expensive offer null |
| documented verification representation | 2 | documented Vast verification string is accepted without boolean alias; complete documented display offer remains within fixed capability |
| documented model representation | 2 | documented display GPU names map to the fixed model allowlist; complete documented display offer remains within fixed capability |
| outbound model names | 2 | outbound API filter uses documented GPU name RTX 4070; outbound API filter uses documented GPU name RTX 4090 |
| documented offer pricing type | 260 | review_or_author accepts owner author; review mode accepts exact head approval by numeric owner; actor_only is explicitly selectable |
| poll permissions guard | 3 | extra workflow permission fails actions: write; extra workflow permission fails contents: write; extra workflow permission fails packages: write |
| attestation dependency guard | 2 | direct attestation dependency is rejected; transitive release dependency is rejected |
| cold-box disk guard | 2 | cold-box disk preflight 22000000000; cold-box disk preflight 59999999999 |
| must-fail control | 2 | box deliberate control exits failure on missing evidence; box harness accepts only a deliberately failed control |
| scheduled cleanup wiring | 3 | scheduled binding reaches Durable Object cleanup; bundled scheduled binding reaches Durable Object cleanup; scheduled cleanup works with a prefix and no public cron route |
| Unicode format sanitizer | 3 | Unicode format control is removed before command prefix 8203; Unicode format control is removed before command prefix 8238; all box lines are prefixed and controls removed "​::error::bad |
| unsupported job syntax | 2 | flow-style release dependency cannot be silently ignored; quoted release job cannot bypass structural parsing |
| distinct client refusal | 3 | workflow client exposes budget refusal as a distinct failing state; workflow client exposes missing approval as a distinct failing state; NOT_NEEDED is explicit successful job and commit status |
| terminal client completion | 2 | workflow client exits on a terminal unavailable verdict without another poll; workflow client exits when a pending run completes unavailable |
| public cron isolation | 6 | public request cannot invoke the internal cron route; bundled public request cannot invoke internal cron route; public routing rejects without forwarding /rent |
| automatic base allowlist | 3 | automatic non-master base is NOT_ELIGIBLE integration/candidate; automatic non-master base is NOT_ELIGIBLE another-pr; missing API base never trusts an OIDC base claim |
| docs-only exclusion result | 5 | complete exclusion list is NOT_NEEDED README.md; complete exclusion list is NOT_NEEDED docs/design.md,traces/evidence.txt,archive/old.rs,LICENSE-MIT,.github/ISSUE_TEMPLATE/bug.yml; complete exclusion list is NOT_NEEDED docs/guide.md,nested/notes.md |
| unknown file-list skip | 5 | incomplete or unknown file metadata runs empty; incomplete or unknown file metadata runs missing-count; incomplete or unknown file metadata runs truncated |
| canonical path validation | 12 | unlisted path runs despite adjacent docs ci/vast-broker/README.md; unlisted path runs despite adjacent docs ci/vast-broker/test/example.md; rename checks both paths docs/new.md from docs/../src/code.md |
| renamed previous path | 2 | rename checks both paths docs/new.md from src/code.rs; rename checks both paths docs/new.md from docs/../src/code.md |
| cached PR display grant | 2 | same SHA cache names the requesting PR in rent; same SHA cache names the requesting PR in status |
| request bearer transport | 3 | request GitHub bearer used only for read-only GitHub calls /rent; request GitHub bearer used only for read-only GitHub calls /status/123-1; new request replaces prior bearer and cron retains none |
| request bearer retention | 3 | request GitHub bearer used only for read-only GitHub calls /rent; new request replaces prior bearer and cron retains none; request token absent from responses and ledger box output |
| public GitHub fallback | 4 | unusable read bearer falls back to authenticated public facts 401; unusable read bearer falls back to authenticated public facts 403; GitHub public fallback native fetch has no receiver 401 |
| owner branch without PR | 2 | owner dispatch permits branch without a PR owner-pr; owner dispatch permits branch without a PR integration/test |
| immutable API cache | 2 | static GitHub facts have a short token-free cache; static response cache works across request clients without retaining bearers |
| mandatory status writer jobs | 2 | missing status writer job fails the guard pending; missing status writer job fails the guard report |
| token-shaped log redaction | 2 | short legacy token family redaction ghp_; short legacy token family redaction github_pat_ |
| isolated writer permissions | 2 | status writer cannot gain content permission pending; status writer cannot gain content permission report |
