# Own-instance diagnostic logs

[CODE READ: scripts/ci/vast_diagnostic_logs.py] The controller explicitly approved fetching
only this lane's provider-issued S3 instance-log URL. This exception exists in the diagnostic
helper; the Worker's fixed-host rules and the normal Vast API allowlist remain unchanged.
The helper binds the log response to the exact owned/requested instance id, uses only its
result_url, checks HTTPS and an S3 hostname under amazonaws.com, refuses userinfo, unexpected
ports and fragments, follows no redirects, and sends no Authorization/Cookie headers.
The response has a five-MB (5,000,000-byte) cap and a 30-second socket timeout. Signed URLs are never logged.
Remote bytes remain untrusted; the caller scrubs before displaying even a selected excerpt.
No SDK fallback exists.

[MEASURED: 2026-10-23, run v7-diagnostic-mutants-20261023; diagnostic_mutants.json] Twenty-six
focused tests pass after the standalone EXPECTED RED import failure. Ten policy mutations
each have at least two independently named killers, including ownership, host allowlisting,
credential headers, limits, redirects, error secrecy and the separate normal API rule.
Black and the 100-column check pass for the diagnostic source, tests and mutation harness.

[MEASURED: 2026-10-23, run v7-kvm-20261023; startup_missing_contract.json] The native Vast
request_logs response for the owned instance returned an HTTPS s3.amazonaws.com URL.
A capped, unauthenticated, non-redirecting fetch recovered the startup log. Only a scrubbed
missing-contract excerpt is public; the URL and complete remote content remain private.

[MEASURED: 2026-10-23, run v7-diagnostic-mutants-20261023; diagnostic_mutants.json] Two
limit probes deliberately failed when the earlier binary cap exceeded the owner's decimal
five-MB ceiling; the same 26 probes and ten mutations pass with the stricter cap.
