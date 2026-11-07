# v3_display: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 332 files (14.5 MB: 288 .png, 35 .gz, 6 .log, 3 .xz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_display.tar.zst`, 4935292 bytes, sha256 `0fbe724b32db4cbbb1d99ed07b1d80fe55796c62ee4e67b214764ac50827a689`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_display.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_display`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_display.tar.zst && echo "0fbe724b32db4cbbb1d99ed07b1d80fe55796c62ee4e67b214764ac50827a689  v3_display.tar.zst" | sha256sum -c`, then `zstd -dc v3_display.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_display/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
