# Rental retirement — 2026-10-05

**STATUS: RESEARCH, 2026-10-05.** The owner authorized retiring all rentals used
by this chat once their useful text evidence was preserved. This branch adds
no product change and makes no new hardware compatibility claim.

## 54049598 — completed Linux candidate work

All 11 checkout worktrees had no tracked modifications. Untracked files were
logs only. `54049598/text-records.jsonl.gz` preserves 1,910 text files from
`/root/prov`, `/workspace/bench` and the main checkout's guest-boot traces
(138,018,550 uncompressed source bytes). `worktree-text-records.jsonl.gz` adds
44 untracked text files from other worktrees (12,885,945 bytes). Each JSONL
file record includes its source path, exact UTF-8 text, byte size and SHA-256;
the worktree listings and Git status are included. No binaries or disk images
were returned. These are historical raw logs, not a rerun or promotion. The
already reviewed candidate-2 conclusions live in
`traces/v3_candidates/cand2_20261004/` on `codex/candidate-2-2026-10-04`.

## 54159260 — native Windows RTX 3060

The observer was stopped and the remaining FIFO drained before retirement.
Only UTF-8 text exports/metadata were copied to the controller. The existing
exporter and collector hashes were checked before use; collection did not
overwrite the original trace. Both JSONL exports validate locally with
`tools/windows-gsp-trace/decode.py --jsonl --max-input-mib 128` and reconstruct
the original capture hashes (without needing to copy binary traces).

- Original capture: 1,840 records, 15 observed missing messages; zero collector
  drops and zero remaining FIFO bytes in its final sidecar.
- Retirement drain: 3,827 records, 146,851 dropped messages before drainage;
  final FIFO empty. This long-idle observer is explicitly **lossy**.
- Neither contains a GFX_POOL_QUERY_SIZE observation or successful pair.
- The original collector exit marker is absent. The retirement wrapper also
  stopped on the collector's expected stderr loss warning because Windows
  PowerShell treated it as `NativeCommandError` with ErrorActionPreference Stop.
  Its trace and final stats were already written. A separate export-only script
  verified/exported those existing files, disabled the startup task, and stopped
  the observer. No successful collector process exit is inferred from that.
- Neither sample establishes complete initialization, OS exclusivity, or
  cross-GPU/driver behavior. See the decoded JSON and exact wrapper outputs.

All evidence was pushed before deleting either rental. Deletion receipts and
an inventory confirming absence are added after the API accepts deletion.
