# walk_wake_20261103: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 7 files (5.2 MB: 4 .png, 2 .gz, 1 .tgz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `walk_wake_20261103.tar.zst`, 3266701 bytes, sha256 `587ad182054b4cf968982872a7debbd0ed47250d34de972a88918b4c5ea6e273`
- Public URL: `https://logs.kayfabe.dev/evidence/walk_wake_20261103.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py walk_wake_20261103`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/walk_wake_20261103.tar.zst && echo "587ad182054b4cf968982872a7debbd0ed47250d34de972a88918b4c5ea6e273  walk_wake_20261103.tar.zst" | sha256sum -c`, then `zstd -dc walk_wake_20261103.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/walk_wake_20261103/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
