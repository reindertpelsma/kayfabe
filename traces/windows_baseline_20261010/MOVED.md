# windows_baseline_20261010: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (4.3 MB: 6 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `windows_baseline_20261010.tar.zst`, 4235554 bytes, sha256 `e2d337d68f9a5704d5feea252ddaabee6e8c24d45b70906ab98e2b0a39e8c86c`
- Public URL: `https://logs.kayfabe.dev/evidence/windows_baseline_20261010.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py windows_baseline_20261010`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/windows_baseline_20261010.tar.zst && echo "e2d337d68f9a5704d5feea252ddaabee6e8c24d45b70906ab98e2b0a39e8c86c  windows_baseline_20261010.tar.zst" | sha256sum -c`, then `zstd -dc windows_baseline_20261010.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/windows_baseline_20261010/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
