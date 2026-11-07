# v3_windows: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 4 files (0.1 MB: 4 .png) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_windows.tar.zst`, 108003 bytes, sha256 `fa6c2d1411f377d8641279843d5e7108a9e25a96f6b61ce6dd5d882c92be94f8`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_windows.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_windows`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_windows.tar.zst && echo "fa6c2d1411f377d8641279843d5e7108a9e25a96f6b61ce6dd5d882c92be94f8  v3_windows.tar.zst" | sha256sum -c`, then `zstd -dc v3_windows.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_windows/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
