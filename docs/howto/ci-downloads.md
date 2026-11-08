# Downloading pinned CI data

The pinned setup-cache downloader and the bench-data helper's `--out` mode keep
their existing single-stream transfer path by default (`KF_FETCH_STREAMS=1`).
Leave the variable unset for the same behavior and command line.

Set `KF_FETCH_STREAMS=8` to explicitly opt in to parallel downloads for controlled
trials; other counts from 2 through 256 are also explicit opt-ins. Parallel
throughput on Vast and compatibility with the live Worker route are UNMEASURED
for this tool. The coordinator will qualify them before changing the production
default in a later PR.
The bench-data helper's `--extract` mode still streams the archive directly into
its extractor.

For a pinned file, you can also run the downloader directly:

```bash
printf '%s\n' "$url" | python3 -I scripts/ci/kf_fetch.py - "$out" \
  --sha256 "$sha256" --streams 8 --chunk-mb 16
```

When you also know the pinned byte count, add `--size BYTES` to reject a size
mismatch before allocating the output. Use `--follow-redirect` if the endpoint redirects. Signed URLs belong on stdin,
so they do not appear in process arguments. Keep shell tracing off around them.
For an authenticated bench-data route, also pass `--worker-auth` with the run's
existing `KF_RUN_TOKEN` and optional paired `KF_ACCESS_ID`/`KF_ACCESS_SECRET`
environment values. Those headers are sent only to the original endpoint.

Re-run the same command after interruption to resume. Keep the output and its
`.fetch-state.json` sidecar together; `--state FILE` chooses a different sidecar.
Completed chunks are checked against the local file before reuse, and the final
file must match the SHA-256 you supplied. A server without range support uses one
stream and restarts an interrupted transfer.

`FETCH_PROGRESS` reports completed bytes and the total. `FETCH_DONE` means the
final hash matched. `FETCH_FAIL` names NETWORK, HASH, SIZE or AUTH. On a hash
failure, the standalone downloader prints `FETCH_BAD_HASH`, keeps the bytes for
inspection and clears trusted completions so another run downloads them again.
The bench-data wrapper retains its existing cleanup and exit-code behavior.
