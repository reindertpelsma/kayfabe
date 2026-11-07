# v3_cifix: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.0 MB: 1 .tgz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_cifix.tar.zst`, 15632 bytes, sha256 `43e807af696fcdcf05bd4521a8e79e9465c3d5922236f2bd9f066a799ca34129`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_cifix.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_cifix`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_cifix.tar.zst && echo "43e807af696fcdcf05bd4521a8e79e9465c3d5922236f2bd9f066a799ca34129  v3_cifix.tar.zst" | sha256sum -c`, then `zstd -dc v3_cifix.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_cifix/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
