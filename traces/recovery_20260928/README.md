# Vast recovery evidence, 2026-09-28

**STATUS: LIVE.** Recovered after the cloud chats stopped. These are historical
outputs from untrusted community machines, not a fresh verification run.
The plan and limitations are in `docs/RESUME_2026-09-28.md`.

Each `<instance>.json.gz` is locally generated compressed JSON with:

- `files`: original remote path, original SHA-256, saved SHA-256, size and number
  of credential redactions;
- `objects`: saved SHA-256 → UTF-8 text, deduplicated by content;
- `notice`: the trust boundary.

The adjacent manifest records the archive hash and every saved file's hash.
Paths are metadata; do not extract or execute them blindly. Example for reading
one log without extracting any files:

```sh
gzip -cd traces/recovery_20260928/53004208.json.gz |
  jq -r --arg p /root/jobs/cdp4.log '. as $d | .files[] | select(.path == $p) | $d.objects[.sha256_saved]'
```

| Instance | Selected files | Key material |
|---|---:|---|
| 53004208 | 1290 | app matrix, CDP bisect and minimal CUDA probe, display lane, job scripts as text, raw guest/host logs |
| 53080587 | 493 | Turing later gates/bare suite/thin suite/CUDA ladder, guest and host logs |
| 53091825 | 58 | completed 570/575 driver checks, staging logs, thin/fat guest logs |

No credential redactions were needed in these selected Kayfabe records. The
archive omits provisioned binaries, installers, VM disks and host credentials.
Readable copies of the most useful logs sit beside the bundles. `tu7.log`
inside 53080587's bundle records gates 9/9 and bare suite 30/30 at `d4ff6be8`;
`tus8.log` and `tul6.log` record the subsequent thin suite and CUDA ladder.

The five missing source commits were recovered independently from patches on
`recovery/vast-tuwork-2026-09-28`; the recovery commit IDs differ, but the tree
IDs at every step match. No executables were copied back from Vast.
