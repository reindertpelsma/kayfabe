# Thin Vast payload

[CODE READ: vast_payload.sh, vast_payload.py] The generic onstart invokes
`bash scripts/ci/vast_payload.sh <sha> <nonce>` from the exact approved checkout.
The payload uses the existing disposable-box flows, rather than another provisioning path:

1. provision_full.sh: fresh box, current host driver, bench tree and this revision's kf3.
2. cuda_ladder.sh host: cup2/cup3/cup8/cup8bench, one repetition, before guest workloads.
3. merge_check.sh: every kf-* crate test, gates, rebuild, raw and thin suites, birth census.
4. cuda_ladder.sh guest: the existing fat guest and unchanged ladder hooks, four rungs.

[CODE READ: vast_payload.py] Helpers that fetch origin see only this approved local SHA.
The adapter validates exact revision and the full RC/READY/EXIT/PASS evidence, streams stage
logs, writes per-stage duration lines, and atomically preserves finished rows in
/workspace/kf-result.json. The PR #104 reporter supplies the run identity and heartbeats.
A first red cancels later work. Soft-deadline expiry records FAIL/timeout with finished rows.
No run/account credential is required by the payload; the reporter owns the run capability.

[CODE READ: vast_payload_hooks.sh, vast_payload.py xid_ok] The existing host_xid_watch is
used both per raw/guest arm and per full stage. One raw victim_release missing-page fault at
0xa11000000 is the only first-version admission, with exact PDE/write type and a CE client.
All other new records refuse. S4's exact-channel guard has one replacement point, xid_ok;
the raw marker currently carries the VA, not its channel. No per-die allowance is introduced.

[CODE READ: test_vast_payload.py, test_vast_payload_v7.py, payload_mutants.py] GPU-free checks:
`python3 -m unittest discover -s scripts/ci -p 'test_vast_payload*.py'`
and `python3 scripts/ci/payload_mutants.py`. Black uses line length 100; shellcheck covers both
shell entrypoints. [Evidence](../../traces/vast_payload_v7_20261018/README.md).

[CODE READ: vast_report.py load_provider_env] The reporter's check-env step supports Vast VMs
that persist contract values in /etc/environment instead of exporting them to a non-login
startup process. Only missing protocol names are parsed as data; process values win and
unrelated provider/account credentials are never imported. The daemon still uses the validated
root-only env file. [Recovery evidence](../../traces/vast_payload_v7_20261018/PROVIDER_ENV.md).

[CODE READ: vast_payload.py provision_grade] Cargo outputs stay in the approved checkout's
`target/`, where the existing bench and fast-guest helpers resolve their clients. Hardware is
queried again before grading provisioning, including its failure path. The live run exposed
both integration defects; their fixes have red/green and mutation coverage but no fresh VM run.
[Final live evidence](../../traces/vast_payload_v7_20261018/LIVE_RUN.md).
