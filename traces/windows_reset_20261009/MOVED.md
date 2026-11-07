# windows_reset_20261009: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (4.5 MB: 5 .gz, 1 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `windows_reset_20261009.tar.zst`, 4336956 bytes, sha256 `01b19da1efacdf34438443a217a93faa192f17ba49c03d13d43160f249484c86`
- Public URL: `https://logs.kayfabe.dev/evidence/windows_reset_20261009.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py windows_reset_20261009`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/windows_reset_20261009.tar.zst && echo "01b19da1efacdf34438443a217a93faa192f17ba49c03d13d43160f249484c86  windows_reset_20261009.tar.zst" | sha256sum -c`, then `zstd -dc windows_reset_20261009.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/windows_reset_20261009/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
