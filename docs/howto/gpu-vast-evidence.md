# What a failed GPU run leaves behind

CODE READ (`scripts/ci/vast_evidence.py`, `ci/vast-broker/src/evidence.js`, 2026-11-06; GPU-free tests only, NOT DEPLOYED, no real run has produced a bundle): a rented GPU box is destroyed shortly after its result, and the job log shows a bounded part of the box log. So that the cause of a failed run is not lost with the box, the box builds one bounded, scrubbed evidence bundle at the end of every run (pass or fail) and uploads it, once, to a private bucket.

- The bundle holds the gate and provision evidence, the host `dmesg`, `free`, `df`, the qemu and swtpm processes, the head of `nvidia-smi -q`, the tail of the box log and the newest screenshots of the Windows VM. It is at most 8 MiB compressed; when it would be larger, the oldest screenshots and the bulkiest logs are dropped first and the log tail is shortened last. GPU UUIDs, serial numbers, MAC addresses and public IP addresses are replaced; a bundle that contains a run token, the broker address, a password, an ssh key, a Vast key or a presigned URL is not uploaded at all.
- You can tell from the end of the job log whether a bundle exists: `evidence uploaded: <run>/<attempt> <bytes> sha256 <12 hex>` means it was stored; `KF_EVIDENCE_REFUSED` or `KF_EVIDENCE_SKIPPED` says why it was not. Neither changes the verdict of the run.
- The bundle is private. It is kept for 30 days and only the repository owner can read it, with `scripts/ci/fetch_evidence.py` or `wrangler r2 object get`. It is never linked from the status API, the site or the job summary.
- Treat a downloaded bundle as untrusted data from a machine that ran test code: read the files, do not run anything from it.

The details (what is kept first, what is scrubbed and scanned, the upload protocol, the bucket and its retention rule) are in the [payload notes](../../scripts/ci/VAST_PAYLOAD.md) and in the [broker deployment notes](../../ci/vast-broker/DEPLOY.md).
