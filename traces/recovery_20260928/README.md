# Recovered CDP evidence (2026-09-28) — the two readable logs only

Copied onto master 2026-09-30 so `docs/design/V3_APP_MATRIX.md` §R3 can cite them from master.
The full recovery bundle (per-box `*.json.gz` archives with the CDP probe source and every recovered
log, plus manifests) stays on branch `recovery/resume-2026-09-28`, directory `traces/recovery_20260928/`,
described by that branch's `docs/RESUME_2026-09-28.md`.

- `53004208-cdp4.log.txt` — the last bisect/probe job on vast 53004208 (RTX 3060): `cdpSimpleQuicksort`
  passes at `0667b784` and times out at `56032c46` (the MC_SERVICE_INTERRUPTS completion fix); the
  dedicated probe reports `child_ran=0` in the guest.
- `53004208-cdp-host.log.txt` — the bare-metal control on the same box: the child runs and returns
  the expected data.
