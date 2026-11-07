**STATUS: LIVE, 2026-11-07.** CODE READ: GPU-free downloader and merged pinned-download integrations; hardware throughput is unqualified.

# Parallel CI downloads

## Evidence and limits

MEASURED, owner-provided dev-box measurement of 2026-11-07: one stream delivered
55.6 MB/s (445 Mbps); eight range streams delivered 223 MB/s aggregate (1.78 Gbps)
over a 2 GiB private bench object. This lane did not repeat that measurement or
contact the Worker, Cloudflare/R2, a GPU host or Vast.

UNMEASURED: eight-stream throughput on a Vast box, including this implementation's
checkpoint and final-hash costs. The coordinator must measure it on an authorized
box. INFERRED: extrapolating the dev-box figure to a 100 GB download on Vast could
be wrong because the network, disk, hashing and object service differ.

CODE READ: prior download mechanisms are the resumable curl loop in
`scripts/bench/windows/win_vm.sh`, the two-step resolver/download in
`scripts/bench/box/fetch_bench_data.sh`, and the pinned Python download in
`scripts/ci/vast_setup.py`. The lane's stdlib-only and isolated-Python requirements
rule out adding an external parallel downloader dependency.

## Implementation contract

CODE READ (`scripts/ci/kf_fetch.py`):

- Invocation: `python3 -I scripts/ci/kf_fetch.py URL OUT --sha256 HEX --streams N --chunk-mb M [--state FILE] [--follow-redirect]`.
  Defaults are eight streams and 16 MiB chunks. `URL=-` reads a URL line from stdin,
  keeping a signed locator out of process arguments. Optional `--size BYTES` rejects
  a metadata size mismatch before opening/preallocating OUT. `--worker-auth` reads the
  existing run/Access environment variables for the original endpoint only.
- A `GET Range: bytes=0-0` probe obtains size, ETag and actual range behavior. It
  works with Worker routes that accept GET rather than HEAD. A 200 response with
  Content-Length selects one whole-object stream; a 206 must give an exact
  Content-Range. The tool refuses an unknown size or a compressed representation.
- Redirects require the explicit switch. At most one hop is followed. Every chunk
  then uses the final locator. HTTPS cannot redirect to HTTP. Proxy environment
  settings are ignored, and authentication headers are never forwarded to a
  redirect destination, including a destination on the original host.
- Only an expiry-indicating 403 triggers re-resolution through the original route.
  The refresh lock and URL generation share each new locator across workers.
  At most two re-resolutions are attempted for the entire invocation. Each
  redirect resolution makes one origin request; an origin 500/reset fails that
  resolution without spending further mints. Only final-object requests retry. An ordinary
  AccessDenied/401 does not trigger an expiry refresh. A refresh that changes size,
  ETag or range support fails SIZE, preserving the partial download for a fresh run.
- Five attempts per chunk use exponential backoff starting at 250 ms. Requests
  time out after 30 seconds without socket progress. Each response must match its
  exact range, ETag, representation and length. Strong ETags also use If-Match.
- OUT is preallocated (posix_fallocate where supported, otherwise ftruncate).
  Threads write disjoint offsets with pwrite, handling partial writes. Network
  buffers are bounded at one MiB per worker, independently of object/chunk size;
  bookkeeping scales with the number of chunks. Only N worker futures are queued.
- The default checkpoint is `OUT.fetch-state.json`. It binds size, ETag, chunk size,
  expected hash and a hash of the original endpoint without its query. A rotated
  signature can therefore resume the same object. It stores chunk indices and
  chunk hashes, never the locator or credentials. Output/state locks reject
  simultaneous writers. Data is fsynced before an atomic, fsynced checkpoint rename.
- Before preallocation on resume, each saved chunk must lie within the existing
  file's length and its local bytes must match its saved hash. A changed size or
  ETag restarts; malformed state is discarded. A saved hash is not authority to
  skip the final SHA-256 over OUT. A bad final hash preserves OUT and the checkpoint
  file, clears trusted completions for a future retry, prints FETCH_BAD_HASH and
  exits nonzero. The tool itself deletes no downloaded bytes on hash failure.
- FETCH_PROGRESS reports completed bytes, total bytes and MBps; FETCH_DONE reports
  elapsed seconds, MBps and stream count. MBps is newly completed bytes per elapsed
  invocation time, including probe, resume verification and final hashing; it is
  not a network-only rate. Error lines use FETCH_FAIL class=NETWORK|HASH|SIZE|AUTH
  and exit statuses 1/3/2/4 respectively. Only sanitized host and object basename
  are logged, never a URL, query, response body or exception text.

## Merged integrations and preserved verdicts

CODE READ: `KF_FETCH_STREAMS` defaults to 8 in both integrated callers; 1 selects
their established transfer path. A single-stream standalone fetcher also works.

CODE READ: `scripts/bench/box/fetch_bench_data.sh --out` retains its existing outer
attempt loop and Worker mint budget (one final URL per attempt, at most three
attempts). The fetcher receives that final locator on stdin; it cannot call the
Worker. Chunk retries reuse it. Expiry requires the existing outer retry to mint
another URL, and the query-independent checkpoint permits resume. Hash mismatch
still skips network retries, deletes the helper's `.part` and exits 3; Worker
refusal still exits 4; successful bytes are renamed only after the helper's pin
check. The helper's contract deliberately differs from the standalone tool's
retain-on-hash-failure behavior. `--extract` keeps its streaming curl/tee/zstd/tar
pipeline and disk use; parallel ranges require an on-disk compressed object.

CODE READ: `scripts/ci/kf-box-blob-url` is a locator resolver, not a byte downloader.
Its existing scoped authentication, 299-second limit, 401 retries and JSON output
are unchanged. Its consumer `scripts/ci/vast_setup.py` validates the locator's
HTTPS host, asset name and size before invoking the fetcher without redirects or
headers. It passes the pinned size with `--size` so remote metadata cannot enlarge
preallocation beyond the existing asset bound. It rechecks pinned size/hash before rename and preserves CacheRefused
and partial-file cleanup on failure. No new credentials are required.

CODE READ: the merged Linux apps path (`scripts/bench/box/apps_gate.sh`,
`scripts/apps/provision_guest_apps.sh`) provisions the app disk locally. The Ubuntu
base image in `scripts/bench/provision_bench_tree.sh` is fetched from a moving
`noble/current` URL without a repository hash pin. There is no merged pinned
prebuilt Linux-apps image download to switch. DESIGN INTENT: first supply a pinned
image manifest and its existing verdict contract; then use the same stdin/pin API.
This lane does not invent a hash or change image provenance policy.

## Integration handoff for open PRs

CODE READ, GitHub PR file lists checked 2026-11-07: #291, #293 and #294 were open.
This lane leaves their owned files untouched, including `win_vm.sh`,
`vast_payload.py`, `.github/workflows/ci.yml`, `.github/workflows/vast-box-scripts.yml`
and `docs/DOWNLOADS.md`.

DESIGN INTENT for #293/#294 (`scripts/bench/windows/win_vm.sh`): replace only the
host-side curl transfer in `cmd_fetch` with the stdin invocation below, using its
existing sum and `.part` path. Keep its eight-attempt outer loop, local pinned
size/hash verdict and final rename. A return code of 3 is a pinned-data failure,
not curl's code 33 (range unsupported). The fetcher handles range fallback itself;
do not preserve the curl-specific rc=33 deletion rule around the Python command.
Do not log `$url`; log the object filename. Keep Windows guest downloads out of scope.

```bash
printf '%s\n' "$url" | python3 -I "$REPO/scripts/ci/kf_fetch.py" - "$CACHE/$f.part" \
  --sha256 "$sum" --size "$bytes" --streams "${KF_FETCH_STREAMS:-8}" --chunk-mb 16 --follow-redirect
```

DESIGN INTENT for #291: use the upgraded helper's file mode where a compressed
archive is already staged. Keep `--extract` streaming where avoiding an archive
copy is part of the stage's disk budget. A future parallel extraction change must
budget temporary disk space and preserve the stage's verdict/retry behavior.

DESIGN INTENT for a direct Worker caller: pass the original bench-data route on
stdin with `--follow-redirect --worker-auth`. Reuse one invocation for the complete
object so that its shared two-refresh budget applies. Avoid an additional outer
mint loop that could exceed the Worker's four-URL allowance. The run token and
Access pair stay on the original request only.

## Verification and falsifiers

MEASURED, local loopback only, 2026-11-07: 47 fetcher tests and 18 integration tests
passed; all 24 fetch mutations have two distinct named killers. Existing
setup-cache tests passed 43 before and 43 after; the bench helper's established
single-stream/extraction regression passed 49 checks. Exact source hashes, expanded test
counts and named mutation killers are in
[the local verification record](../../traces/parallel_fetch_20261107/RESULTS.json).
No Vast download rate was measured.

MEASURED, J4 local attempts `J4-existing-mutants` and `J4-existing-mutants-retry`,
2026-11-07 (outcomes in the local verification record above): the unchanged
`scripts/ci/setup_cache_mutants.py` stopped at its existing 45-second `base_env`
baseline limit, then at the existing 45-second `readiness` limit on the retry.
Those local attempts establish no full legacy mutation result; no timeout or
pass rule was changed.

MEASURED, GitHub Actions run
[42388976539](https://github.com/reindertpelsma/kayfabe/actions/runs/42388976539),
2026-11-07 at commit `39127e25b15bd11b39c20828e9a12486cc478772`: the full
setup-cache mutation step and the run-report mutation step completed successfully.
That CI run qualifies those harnesses at that revision; the named local timeout
attempts remain separate observations.

DESIGN INTENT: `.github/workflows/parallel-fetch.yml` runs the local HTTP tests,
existing setup-cache contracts and mutation harness. It needs no GPU, VM,
credentials or external object download. It does not modify the open PRs' workflows.

CODE READ: `scripts/ci/kf_fetch_mutants.py` requires a green full baseline and two
distinct named failing tests per exact-once source mutation. It mutates private
copies, never the checkout or a ratchet baseline. Mutations cover final hash,
retry, range, redirect, expiry budget, credential forwarding, resume identity,
resume bounds/content, checkpoint persistence, offset/short writes, log secrecy
and the integrations' transfer and verdict paths.

DESIGN INTENT (falsifiers stated before the fixture runs): a second initial Worker
mint, credentials on the redirected endpoint, skipping a chunk outside the old
file, accepting a mismatched final hash, or a surviving/once-killed mutant refutes
the claimed corresponding contract. Local transfer tests do not qualify remote
service behavior, throughput, power-loss durability, or performance on a large image.
