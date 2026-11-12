# Custom hosts: Worker, daemon and seeder contract

**STATUS: DESIGN-ONLY 2026-11-12** (coordinator, from owner decisions; sources: memory project_platform_testing_worker_20261103, RELEASE_SEQUENCE.md "CUSTOM HOSTS", "Seeder address rule", box protocol 7 in ci/vast-broker/docs/BOX_PROTOCOL.md). Shared contract for three work packages: W1 Worker (Durable Object + admin), W2 daemon (scripts/kfhost), W3 seeder (nginx template + probe + helper). Nothing here is deployed.

## Principles (owner decisions)
- Custom hosts are enrolled boxes that are NOT rented from Vast. The daemon NEVER runs on Vast boxes (Vast keeps API + per-run token).
- Transport: outbound WebSocket from the daemon to a Durable Object (one DO per box, hibernation API). Auth: Cloudflare Access service token (anti-bot gate only) + a per-box token (distinct from run tokens). The socket carries liveness, capabilities, job offers and (owned boxes only) remote commands. Logs/progress/results/evidence travel over the EXISTING HTTPS box protocol (POST /report/<run>, /run-report, /evidence, /blob-url ...) with the per-run token carried in the offer.
- Profiles are assigned SERVER-SIDE by the admin, never self-declared by the box:
  `owned` (full access, driver swaps, remote commands allowed), `container` (container with /dev/kvm + /dev/nvidia*, host driver fixed, no remote commands), `borrowed` (distrusts us: declarative assignments only; the daemon fetches the commit from the PUBLIC repo, verifies commit SHA and tree key, runs standalone/manual mode: no system changes, official URLs only, no private CF assets, no remote shell).
- Results from custom hosts are unattested tripwires like all others; they may satisfy requirements only when the admin enabled the box for that tier ("reproduced" tier = rented/controlled/enrolled; community results never count).
- Selection: custom hosts that match the requirements and are enabled are preferred over Vast; one run per box at a time; availability window / `opportunistic` flag.
- A box NEVER supplies an address the Worker contacts. Seeder addresses come from the Vast API (Vast boxes) or from admin configuration (custom boxes: remote IP, external TCP port, inner TCP port; the IP is prefilled from the connect address but editable).

## WebSocket messages (JSON, one object per frame, `v: 1`)
daemon -> DO:
- `hello {v, box_id, version, capabilities}` first frame after upgrade. capabilities = {gpu_name, compute_cap, vram_gb, driver, module_flavor: open|proprietary, kvm: bool, cpu_model, cores, ram_gb, disk_free_gb, download_mbps (measured, optional), display: bool, can_swap_driver: bool, container: bool, os, kernel}. The DO stores them; the admin profile decides what they are allowed to do.
- `heartbeat {ts, state: idle|busy, run_id?}` every 15 s.
- `ack {offer_id}` / `nack {offer_id, reason}`.
- `seed_register {offer_id|run_id, artifact: {name, sha256, size}, path}`; the DO answers after probing (below).
- `cmd_output {cmd_id, seq, stream: stdout|stderr, data, eof?}` and `cmd_done {cmd_id, rc}` (owned boxes only).
DO -> daemon:
- `welcome {v, box_id, profile, heartbeat_s}` (profile is informative; the daemon enforces the strictest of profile and local config).
- `offer {offer_id, request_id, run_id, run_token, assignment: {repo, commit, tree_key, stages?, requirements?, env?}, deadline}`; `borrowed` offers contain ONLY repo/commit/tree_key/stage names from a fixed allowlist, never scripts or commands.
- `cancel {offer_id}`.
- `seed_ack {ok, reason?}`.
- `cmd {cmd_id, argv|shell}` (owned only; refused by the daemon on other profiles).
- `ping`/close codes: 4401 bad token, 4403 box disabled, 4409 replaced by a newer connection.

## Seeder probe (W3 + W1)
- Admin config per box: `seeder: {enabled, remote_ip, external_port, inner_port}`. For Vast instances the Worker uses the IP + mapped port that the Vast API reports for the instance.
- Registration probe: the Worker fetches `http://<hostname>:<port>/<token>/probe.txt` (hostname = an IP-encoding mapper name of the configured IP, see the MEASURED note below; never the raw IP) (a small dummy file served by the box's nginx under the same token path as the artifact) and compares its fixed content; success => `seed_ack {ok: true}` and the box may be a seeder for the session; failure => `ok: false` and the box is treated as unable to open a port. Timeouts <= 5 s per name, one attempt per name, no redirects, only http to the configured address, never an address from the box.
- Consumers receive the artifact record from the Worker: `url` (`http://ip:port/<token>/<name>`), `fallback_url` (the hostname form of the first mapper that passed the probe, so a name known to work from Cloudflare's side; the sslip.io form if the probe did not apply; for `kf_fetch.py --fallback-url`, because boxes behind http proxies, DNS proxies or NAT64 may only reach the hostname), `sha256` and `size`; all four are Worker-published and never taken from the seeder's responses; they verify the hash and fall back to a local build at the deadline.
- [MEASURED 2026-11-12, coordinator] A Worker `fetch()` of a RAW IP (`http://92.67.98.9:25493/...`) returns 403 `error code: 1003` in under 2 ms: Cloudflare blocks direct IP access. A HOSTNAME that resolves to the IP works on a non-standard port: `http://92-67-98-9.sslip.io:25493/<token>/probe.txt` answered 200 with the exact body in about 100 ms; `92.67.98.9.nip.io` also works (about 900 ms); a wrong token answers 404; a closed port answers Cloudflare 522 after about 19 s. Therefore the probe host is always a hostname from an IP-encoding DNS mapper. [MEASURED 2026-11-12 with dig, coordinator] sslip.io and nip.io share the same nameservers (one operator), so nip.io is no independent second path and is not used; `a-b-c-d.traefik.me` (Traefik Labs, its own nameservers) resolves correctly via 1.1.1.1 and 8.8.8.8; local-ip.sh also works (listed, disabled); cloud-ip.cc is not an IP-encoding mapper and xip.io is dead. [NOT YET MEASURED] traefik.me through a real Worker fetch (the coordinator measures it in integration). The probe tries `<a-b-c-d>.sslip.io`, then `<a-b-c-d>.traefik.me` (the `MAPPERS` constant in `hosts.js`; a third name is a one-line change): one attempt per name, each hard-bounded at 5 s, and the first name that returns the exact probe text wins; if both fail the box is "cannot open a port" for the session. There is no admin-chosen or own-zone name (owner ruling 2026-11-12: the http connection is untrusted anyway; the Worker publishes url, sha256 and size, and consumers stage to `.unverified` and discard on a size or hash violation). The IP must be a public dotted-quad (no IPv6 for now; no private, loopback, link-local or CGNAT range). The probe function takes its fetcher as an argument.

## Admin API (extends the existing owner admin auth; JSON)
`GET /admin/hosts`, `PUT /admin/hosts/<box_id>` {enabled, profile, tiers: [small|middle|large], seeder, window?, opportunistic?, notes}, `POST /admin/hosts/<box_id>/token` (mint/rotate the per-box token; the token is shown once, stored hashed), `DELETE /admin/hosts/<box_id>`, `POST /admin/hosts/<box_id>/cmd` (owned only; command log kept in the DO, bounded).

## Selection (planner)
`hostsFor(requirements)`: enabled boxes, connected (heartbeat < 45 s), idle, profile allows the tier, capabilities satisfy the requirement (die/family, driver needs vs can_swap_driver, kvm, display), inside the availability window; ordered: non-Vast first, then lowest-cost Vast fallback (existing logic).

## Boundaries
- No secrets in repo; tokens only in env/DO storage hashed; the daemon holds a per-box token + Access service token in a 0600 file it reads at start.
- Python rule: logic in Python (shell is glue only); the daemon runs in a venv created by an explicit setup command; deps: `websockets` pinned with hashes.
- GPU-free tests everywhere: fake WS server for the daemon, workerd/miniflare for the DO, local HTTP server for the probe.

## Implementation notes (W1, Worker side; code in `ci/vast-broker/src/hosts.js` and `hostbox.js`)
- Wire: frames are `{type, v?, ...}`; the discriminator is `type`. `v` is `1` or absent on daemon frames (the daemon omits it); the Worker's frames carry `v: 1`.
- Upgrade route: `GET /hosts/<box_id>/ws` (behind the broker path prefix, if one is set) with `Authorization: Bearer <box token>`. The gateway forwards it to the box's Durable Object, which compares the token's SHA-256 against the stored hash in constant time. Unknown box, no token minted and a wrong token are all `4401`; a valid token on a disabled box is `4403`; a newer connection closes the older one with `4409`; 45 s without a frame closes with `4408`.
- Admin (owner Access identity, in `ci/vast-admin`): `GET /api/hosts`, `GET|PUT|DELETE /api/hosts/<box_id>`, `POST /api/hosts/<box_id>/token`, `POST /api/hosts/<box_id>/cmd`; each is forwarded to the DO as `/admin/hosts...` on the internal origin. A reserved DO instance `__index__` lists the box ids (at most 64).
- Config per box: `enabled`, `profile` (`owned|container|borrowed`, admin only), `tiers`, `seeder {enabled, remote_ip, external_port, inner_port}`, `window {start, end}` in UTC (`HH:MM`, wraps midnight), `opportunistic` (ordered after non-opportunistic boxes), `notes`.
- Offers (internal origin, called by the planner): `borrowed` assignments contain only `repo`, `commit`, `tree_key` and `stages` from the Worker's own stage names; `owned` and `container` may add bounded `requirements` and `KF_*` `env`. The run token is minted per offer and never stored. An offer is idempotent by `request_id`: the same request again after an unacked offer re-sends the same `offer_id` with a fresh token; once acked it answers `duplicate`. An ack that arrives after the ack timeout still counts until the offer's deadline (the daemon acks after fetching and verifying the commit). After an ack, the box is free again when it reports idle without a run id for more than 35 s.
- Selection: `hostsFor(requirements, snapshots, now)` in `hosts.js` is pure; `hostSnapshots(env)` and `offerToHost(env, boxId, offer)` in `hostbox.js` are the planner's seams. An empty result means "fall back to Vast". The planner itself is not changed.
- Not deployed. Adding the Durable Object class needs the `v2` migration in `wrangler.toml`; the bundle hash pin (`deployment.json`, `DEPLOYED_BUNDLE_SHA256`) must be regenerated by the coordinator.
