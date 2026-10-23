# Fresh KVM acceptance after the cloud-init replay — FAIL/raw_suite

MEASURED on 2026-10-23, run `v8-cloudinit-proof-kvm-20261023`, source
`d196c055ab97e63293804ede0ff354f25a6067ba`: one verified AD102/RTX4090 host, country IL,
compute cap890, vm:true/ssh_direct. No qualifying GA106 offer was available. All profile
requirements stayed forced. Effective0.560035590278 USD/h; compute/storage0.550222222222 USD/h;
full transfer reserve0.029440104167 USD. Independent exact-id reaper armed before create.
[Structured result and timings](live_run.json), [availability counts/thresholds](availability_counts.json),
[pre-recorded falsifier](experiment_plan.json). Country is a preference, never a requirement waiver.

MEASURED on 2026-10-23, run `v8-cloudinit-proof-kvm-20261023`: the production tiny onstart
cloned the immutable approved SHA and launched the repo entry without SSH intervention.
First accepted report121.775s from rent-start; first independent running poll124.429s,
20s sampling. The first report preceded the first blob lookup; both blob requests returned200.
The Worker's running-only blob rule, shipped deadlines, budgets and strict Xid rule were unchanged.

MEASURED on 2026-10-23, run `v8-cloudinit-proof-kvm-20261023`:

| Phase | Result | Evidence limit |
|---|---:|---|
| .deb fetch/hash/extract | 7.027s /159253208 bytes | Combined phase, not pure throughput |
| Vendor fetch/hash/extract | 2.361s /7888026 bytes | Combined phase, not pure throughput |
| Entire cache restore | 20.346s | Includes probes/configuration |
| Provision to READY | 340.059s | Excludes preceding cache; live status capture |
| Host CUDA | 4/4,10.445s | cup2/cup3/cup8/cup8bench |
| Merge bar until first red | 881.209s | Includes tests/gates/build/raw prefix |
| kf tests | 3749 passed,0 failed | Exact approved revision |
| v3 gates | 9/9,gate10=PASS | No gate waiver |
| kf3 build | KF3_RC=0 | Same revision |
| Raw suite | 6 arms passed,then defer-liveness failed | Wrapper rc6;23 arms not run |
| Thin30/census/fat CUDA | NOT REACHED | First-red stop |

CODE READ: the timing adapter uses the existing provision_full, host CUDA ladder, merge_check
and fat CUDA flows. The provision completion row was captured from authenticated /status and
recorded in the operator journal/public PR while running; it rolled out of the final bounded
log tail. The other stage completions are retained in the [bounded excerpt](live_log_excerpt.txt).
MEASURED on 2026-10-23, run `v8-cloudinit-proof-kvm-20261023`: phase durations sum to360.405s,
excluding small inter-stage overhead;
INFERRED from this run: setup meets the roughly900s target.
Separate fresh APT install timing was unavailable; no package-install duration is inferred.

MEASURED on 2026-10-23, run `v8-cloudinit-proof-kvm-20261023`: the runner accepted a locally
validated, SHA/nonce/log-chain-bound FAIL/raw_suite, verified=1. The first non-PASS raw row was
`BARE_CELL_ARM arm=defer-liveness verdict=FAIL ms=16920 rc=6`. Prior raw arms timer, engines,
doorbell-census, gpu-info-sweep, bus-info-sweep and concurrency passed. CODE READ: rc6 is the
wrapper status; the hook can return it for a refused Xid/instrument/command outcome, so it is
not proof of one particular GPU fault. No exemption, timeout change, source patch or restart
was made. This is a reported raw-arm finding with an unverified underlying cause.

MEASURED on 2026-10-23, run `v8-cloudinit-proof-kvm-20261023`: initial watcher hardware was
RTX4090/575.51.03; later watcher/raw rows and the final FAIL reported580.159.04/compute890.
The post-swap failure hardware path therefore ran. The bench reported a5058248-byte musl
client, and raw arms executed the checkout's target/release client at this revision, exercising
the Cargo-path fix. Independent SSH artifact/hardware snapshots were unavailable: both existing
account-registered identities were denied; explicit public-key attach to the exact owned id
returned200 but SSH remained denied. No account key was created, rotated or removed; only the existing public key was sent.
The diagnostic helper's process exit0 did not mean SSH success; its SSH status was255/no data.

MEASURED on 2026-10-23, run `v8-cloudinit-cleanup-20261023`: exact id and exact label absence
were independently reverified through native Vast APIs. Reaper stopped only after absence;
22 real local scheduled triggers occurred during the lease. Controller/tunnel/identity/ingress,
listeners and source gate stopped; no firewall rule was added. Temporary capability/secret
files and local Worker state were deleted. No Cargo target or host lock was created locally.
INFERRED from the recorded price/lifetime plus full transfer reserve in this run: conservative
cost upper bound0.242045705 USD, not an invoice, below stop3/cap4 USD. One lease used; no further
lease is authorised by this instruction. Worker bundle remains
`2402798ce8ab53dcc4db5efca7891b565c0bca7e92723ff0b5050e5bb6e6760f`; nothing deployed or enabled.

MEASURED on 2026-10-23, run `v8-cloud-init-hosted-20261023`: both hosted box-script checks
passed on source d196c055ab97e63293804ede0ff354f25a6067ba ([push39929665645](https://github.com/reindertpelsma/kayfabe/actions/runs/39929665645),
[PR39929672948](https://github.com/reindertpelsma/kayfabe/actions/runs/39929672948)).
The pre-rent325 tests,96 noncanonical mutants with>=2 named killers each, canonical60/60 and
both fresh-container scenarios are in [preflight evidence](preflight.json). General Rust CI is separate.

NOT DONE: overall GPU PASS; remaining23 raw arms, thin30/30, census and fat CUDA; root-cause
attribution/per-arm log retrieval for defer-liveness; independent SSH artifact/hardware proof;
separate fresh APT timing; S4 exact-channel guard; hosted OIDC; billed Cloudflare CPU.
Deferred Worker format2/README/review lows remain. Earlier MANUAL retention proof remains
accepted; this lease did not repeat it. No GPU result is attested: gpu-vast remains a tripwire.
Owner decision: route the raw-arm/guard finding upstream before authorising another full-suite
lease; retain the strict admission rule. No kayfabe crate change belongs to this lane.
