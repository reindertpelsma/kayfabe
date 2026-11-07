# linux_regression_20261010: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (0.9 MB: 2 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `linux_regression_20261010.tar.zst`, 486818 bytes, sha256 `ad68606d1f927e37d5f8c71725e2f3c6c215a50956bb0c6ecbd9a0086d8e33fa`
- Public URL: `https://logs.kayfabe.dev/evidence/linux_regression_20261010.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py linux_regression_20261010`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/linux_regression_20261010.tar.zst && echo "ad68606d1f927e37d5f8c71725e2f3c6c215a50956bb0c6ecbd9a0086d8e33fa  linux_regression_20261010.tar.zst" | sha256sum -c`, then `zstd -dc linux_regression_20261010.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/linux_regression_20261010/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
